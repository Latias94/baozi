use crate::attribute::{AttributeBuffer, AttributePayload, AttributeSemantic};
use crate::canonical_scene::CanonicalSceneData;
use crate::deform::{InfluenceRows, Skin, SkinMatrixSpace, SkinMatrixSpaces};
use crate::derived::{Derived, DerivedAbsenceReason, InputStamp, InputStampBuilder};
use crate::entity::{AuthoredTransform, AuthoredTransformKind};
use crate::extension::ExtensionSchemaId;
use crate::geometry::{LogicalMesh, MeshPrimitive, MorphBoundsPolicy, TopologyKind};
use crate::math::{Aabb, Mat4, Vec3};
use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, SceneSpace, TangentBasisConvention,
    TangentHandednessEncoding, TransformConvention, UvOrigin,
};
use crate::validation_control::SCENE_VALIDATION_POLL_INTERVAL;
use std::convert::Infallible;
use thiserror::Error;

const PRIMITIVE_BOUNDS_DOMAIN: &[u8] = b"baozi.derived.primitive-bounds";
const LOGICAL_MESH_BOUNDS_DOMAIN: &[u8] = b"baozi.derived.logical-mesh-bounds";
const SCENE_BOUNDS_DOMAIN: &[u8] = b"baozi.derived.scene-bounds";
const PRIMITIVE_BOUNDS_SCHEMA_VERSION: u32 = 2;
const LOGICAL_MESH_BOUNDS_SCHEMA_VERSION: u32 = 2;
const SCENE_BOUNDS_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DerivedInputStampError {
    #[error("derived input stamp work accounting overflowed")]
    WorkOverflow,
    #[error("a canonical primitive has no position attribute")]
    MissingPositionAttribute,
    #[error("a canonical position or morph payload is not semantic F32x3 data")]
    InvalidPositionPayload,
    #[error("an authored transform representation is internally inconsistent")]
    InvalidAuthoredTransform,
}

pub(crate) struct DerivedInputStampEvidence {
    stamp: InputStamp,
    work: u64,
}

impl DerivedInputStampEvidence {
    pub(crate) const fn stamp(&self) -> InputStamp {
        self.stamp
    }

    pub(crate) const fn work(&self) -> u64 {
        self.work
    }
}

pub(crate) enum ControlledDerivedInputStampError<E> {
    Stamp(DerivedInputStampError),
    Control(E),
}

impl<E> From<DerivedInputStampError> for ControlledDerivedInputStampError<E> {
    fn from(error: DerivedInputStampError) -> Self {
        Self::Stamp(error)
    }
}

type ControlledStampResult<T, E> = Result<T, ControlledDerivedInputStampError<E>>;

struct StampEncoder<'a, E> {
    builder: InputStampBuilder,
    work: u64,
    charge_work: &'a mut dyn FnMut(u64) -> Result<(), E>,
}

impl<'a, E> StampEncoder<'a, E> {
    fn new(
        domain: &[u8],
        dependency_schema_version: u32,
        charge_work: &'a mut dyn FnMut(u64) -> Result<(), E>,
    ) -> Self {
        Self {
            builder: InputStampBuilder::new(domain, dependency_schema_version),
            work: 0,
            charge_work,
        }
    }

    fn add_work(&mut self, amount: usize) -> ControlledStampResult<(), E> {
        let amount = u64::try_from(amount).map_err(|_| DerivedInputStampError::WorkOverflow)?;
        let next_work = self
            .work
            .checked_add(amount)
            .ok_or(DerivedInputStampError::WorkOverflow)?;
        (self.charge_work)(amount).map_err(ControlledDerivedInputStampError::Control)?;
        self.work = next_work;
        Ok(())
    }

    fn absorb_work(&mut self, amount: u64) -> ControlledStampResult<(), E> {
        self.work = self
            .work
            .checked_add(amount)
            .ok_or(DerivedInputStampError::WorkOverflow)?;
        Ok(())
    }

    fn write_tag(&mut self, value: u8) -> ControlledStampResult<(), E> {
        self.add_work(1)?;
        self.builder.write_tag(value);
        Ok(())
    }

    fn write_u32(&mut self, value: u32) -> ControlledStampResult<(), E> {
        self.add_work(1)?;
        self.builder.write_u32(value);
        Ok(())
    }

    fn write_u64(&mut self, value: u64) -> ControlledStampResult<(), E> {
        self.add_work(1)?;
        self.builder.write_u64(value);
        Ok(())
    }

    fn write_len(&mut self, value: usize) -> ControlledStampResult<(), E> {
        let value = u64::try_from(value).map_err(|_| DerivedInputStampError::WorkOverflow)?;
        self.write_u64(value)
    }

    fn write_f32(&mut self, value: f32) -> ControlledStampResult<(), E> {
        self.add_work(1)?;
        self.builder.write_f32(value);
        Ok(())
    }

    fn write_f64(&mut self, value: f64) -> ControlledStampResult<(), E> {
        self.add_work(1)?;
        self.builder.write_f64(value);
        Ok(())
    }

    fn write_bytes(&mut self, value: &[u8]) -> ControlledStampResult<(), E> {
        self.builder.write_bytes_len(value.len());
        if value.is_empty() {
            self.add_work(1)?;
            return Ok(());
        }
        let chunk_len = usize::try_from(SCENE_VALIDATION_POLL_INTERVAL)
            .map_err(|_| DerivedInputStampError::WorkOverflow)?;
        for chunk in value.chunks(chunk_len) {
            self.add_work(chunk.len())?;
            self.builder.write_raw_bytes(chunk);
        }
        Ok(())
    }

    fn write_vec3(&mut self, value: Vec3) -> ControlledStampResult<(), E> {
        self.write_f32(value.x)?;
        self.write_f32(value.y)?;
        self.write_f32(value.z)
    }

    fn write_aabb(&mut self, value: Aabb) -> ControlledStampResult<(), E> {
        self.write_vec3(value.min)?;
        self.write_vec3(value.max)
    }

    fn finish(self) -> DerivedInputStampEvidence {
        DerivedInputStampEvidence {
            stamp: self.builder.finish(),
            work: self.work,
        }
    }
}

fn unwrap_uncontrolled<T>(
    result: ControlledStampResult<T, Infallible>,
) -> Result<T, DerivedInputStampError> {
    match result {
        Ok(value) => Ok(value),
        Err(ControlledDerivedInputStampError::Stamp(error)) => Err(error),
        Err(ControlledDerivedInputStampError::Control(error)) => match error {},
    }
}

pub(crate) fn primitive_bounds_input_stamp(
    primitive: &MeshPrimitive,
    default_morph_weights: &[f32],
) -> Result<DerivedInputStampEvidence, DerivedInputStampError> {
    unwrap_uncontrolled(primitive_bounds_input_stamp_controlled(
        primitive,
        default_morph_weights,
        &mut |_| Ok(()),
    ))
}

pub(crate) fn primitive_bounds_input_stamp_controlled<E>(
    primitive: &MeshPrimitive,
    default_morph_weights: &[f32],
    charge_work: &mut dyn FnMut(u64) -> Result<(), E>,
) -> ControlledStampResult<DerivedInputStampEvidence, E> {
    let mut encoder = StampEncoder::new(
        PRIMITIVE_BOUNDS_DOMAIN,
        PRIMITIVE_BOUNDS_SCHEMA_VERSION,
        charge_work,
    );
    write_morph_bounds_policy(&mut encoder, primitive.bounds().morph_policy())?;
    encoder.write_u32(primitive.cardinality().vertex_count())?;
    write_topology(&mut encoder, primitive)?;

    let positions = primitive
        .attributes()
        .iter()
        .find(|attribute| attribute.semantic() == &AttributeSemantic::Position)
        .ok_or(DerivedInputStampError::MissingPositionAttribute)?;
    write_vec3_payload(&mut encoder, positions.payload())?;

    if primitive.bounds().morph_policy() == MorphBoundsPolicy::DefaultWeights {
        write_position_morph_inputs(&mut encoder, primitive)?;
        encoder.write_len(default_morph_weights.len())?;
        for weight in default_morph_weights {
            encoder.write_f32(*weight)?;
        }
    }

    Ok(encoder.finish())
}

pub(crate) fn logical_mesh_bounds_input_stamp(
    logical_mesh: &LogicalMesh,
) -> Result<DerivedInputStampEvidence, DerivedInputStampError> {
    unwrap_uncontrolled(logical_mesh_bounds_input_stamp_controlled(
        logical_mesh,
        &mut |_| Ok(()),
    ))
}

pub(crate) fn logical_mesh_bounds_input_stamp_controlled<E>(
    logical_mesh: &LogicalMesh,
    charge_work: &mut dyn FnMut(u64) -> Result<(), E>,
) -> ControlledStampResult<DerivedInputStampEvidence, E> {
    let mut encoder = StampEncoder::new(
        LOGICAL_MESH_BOUNDS_DOMAIN,
        LOGICAL_MESH_BOUNDS_SCHEMA_VERSION,
        charge_work,
    );
    write_morph_bounds_policy(&mut encoder, logical_mesh.bounds().morph_policy())?;
    encoder.write_len(logical_mesh.primitives().len())?;
    for primitive in logical_mesh.primitives() {
        let primitive_stamp = primitive_bounds_input_stamp_controlled(
            primitive,
            logical_mesh.default_morph_weights(),
            encoder.charge_work,
        )?;
        encoder.absorb_work(primitive_stamp.work())?;
        encoder.write_bytes(primitive_stamp.stamp().as_bytes())?;
        write_derived_bounds(&mut encoder, primitive.bounds().derived())?;
        if logical_mesh.bounds().morph_policy() == MorphBoundsPolicy::DefaultWeights {
            write_position_morph_inputs(&mut encoder, primitive)?;
        }
    }

    if logical_mesh.bounds().morph_policy() == MorphBoundsPolicy::DefaultWeights {
        encoder.write_len(logical_mesh.default_morph_weights().len())?;
        for weight in logical_mesh.default_morph_weights() {
            encoder.write_f32(*weight)?;
        }
    }

    Ok(encoder.finish())
}

pub(crate) fn scene_bounds_input_stamp(
    data: &CanonicalSceneData,
) -> Result<DerivedInputStampEvidence, DerivedInputStampError> {
    unwrap_uncontrolled(scene_bounds_input_stamp_controlled(data, &mut |_| Ok(())))
}

pub(crate) fn scene_bounds_input_stamp_controlled<E>(
    data: &CanonicalSceneData,
    charge_work: &mut dyn FnMut(u64) -> Result<(), E>,
) -> ControlledStampResult<DerivedInputStampEvidence, E> {
    let mut encoder = StampEncoder::new(
        SCENE_BOUNDS_DOMAIN,
        SCENE_BOUNDS_SCHEMA_VERSION,
        charge_work,
    );
    write_scene_space(&mut encoder, &data.scene_space)?;
    write_optional_u32(&mut encoder, data.root.map(|root| root.as_u32()))?;

    encoder.write_len(data.nodes.len())?;
    for node in &data.nodes {
        let node = node.as_ref();
        write_authored_transform(&mut encoder, node.authored_transform())?;
        encoder.write_len(node.children().len())?;
        for child in node.children() {
            encoder.write_u32(child.as_u32())?;
        }
    }

    encoder.write_len(data.logical_meshes.len())?;
    for logical_mesh in &data.logical_meshes {
        let logical_mesh = logical_mesh.as_ref();
        let logical_stamp =
            logical_mesh_bounds_input_stamp_controlled(logical_mesh, encoder.charge_work)?;
        encoder.absorb_work(logical_stamp.work())?;
        encoder.write_bytes(logical_stamp.stamp().as_bytes())?;
        write_derived_bounds(&mut encoder, logical_mesh.bounds().derived())?;
        encoder.write_len(logical_mesh.primitives().len())?;
        for primitive in logical_mesh.primitives() {
            let primitive_stamp = primitive_bounds_input_stamp_controlled(
                primitive,
                logical_mesh.default_morph_weights(),
                encoder.charge_work,
            )?;
            encoder.absorb_work(primitive_stamp.work())?;
            encoder.write_bytes(primitive_stamp.stamp().as_bytes())?;
            write_derived_bounds(&mut encoder, primitive.bounds().derived())?;
            write_influence_rows(&mut encoder, primitive.influences())?;
        }
    }

    encoder.write_len(data.skins.len())?;
    for skin in &data.skins {
        write_skin(&mut encoder, skin.as_ref())?;
    }

    encoder.write_len(data.mesh_instances.len())?;
    for instance in &data.mesh_instances {
        let instance = instance.as_ref();
        encoder.write_u32(instance.owner_index().as_u32())?;
        encoder.write_u32(instance.logical_mesh_index().as_u32())?;
        write_optional_u32(
            &mut encoder,
            instance.skin_index().map(|skin| skin.as_u32()),
        )?;
        if let Some(weights) = instance.morph_weight_override() {
            encoder.write_tag(1)?;
            encoder.write_len(weights.len())?;
            for weight in weights {
                encoder.write_f32(*weight)?;
            }
        } else {
            encoder.write_tag(0)?;
        }
    }

    Ok(encoder.finish())
}

fn write_position_morph_inputs<E>(
    encoder: &mut StampEncoder<'_, E>,
    primitive: &MeshPrimitive,
) -> ControlledStampResult<(), E> {
    encoder.write_len(primitive.morph_targets().len())?;
    for target in primitive.morph_targets() {
        encoder.write_u32(target.target().as_u32())?;
        if let Some(delta) = target.delta(crate::deform::MorphDeltaSemantic::Position) {
            encoder.write_tag(1)?;
            write_vec3_payload(encoder, delta.payload())?;
        } else {
            encoder.write_tag(0)?;
        }
    }
    Ok(())
}

fn write_topology<E>(
    encoder: &mut StampEncoder<'_, E>,
    primitive: &MeshPrimitive,
) -> ControlledStampResult<(), E> {
    encoder.write_tag(match primitive.topology().kind() {
        TopologyKind::Points => 0,
        TopologyKind::LineList => 1,
        TopologyKind::LineStrip => 2,
        TopologyKind::LineLoop => 3,
        TopologyKind::TriangleList => 4,
        TopologyKind::TriangleStrip => 5,
        TopologyKind::TriangleFan => 6,
        TopologyKind::Polygons => 7,
    })?;
    if let Some(boundaries) = primitive.topology().polygon_boundaries() {
        encoder.write_len(boundaries.face_corner_counts().len())?;
        for count in boundaries.face_corner_counts() {
            encoder.write_u32(*count)?;
        }
    }
    if let Some(indices) = primitive.index_stream().indices() {
        encoder.write_tag(1)?;
        encoder.write_len(indices.len())?;
        for index in indices {
            encoder.write_u32(*index)?;
        }
    } else {
        encoder.write_tag(0)?;
    }
    Ok(())
}

fn write_vec3_payload<E>(
    encoder: &mut StampEncoder<'_, E>,
    payload: &AttributePayload,
) -> ControlledStampResult<(), E> {
    encoder.write_u32(payload.logical_count())?;
    if let Some(indices) = payload.sparse_indices() {
        encoder.write_tag(1)?;
        encoder.write_len(indices.len())?;
        for index in indices {
            encoder.write_u32(*index)?;
        }
    } else {
        encoder.write_tag(0)?;
    }
    write_vec3_buffer(encoder, payload.explicit_values())?;
    if let Some(default) = payload.default_value() {
        encoder.write_tag(1)?;
        write_vec3_buffer(encoder, default)?;
    } else {
        encoder.write_tag(0)?;
    }
    Ok(())
}

fn write_vec3_buffer<E>(
    encoder: &mut StampEncoder<'_, E>,
    buffer: &AttributeBuffer,
) -> ControlledStampResult<(), E> {
    let values = buffer
        .f32_vec3_view()
        .ok_or(DerivedInputStampError::InvalidPositionPayload)?;
    encoder.write_len(values.len())?;
    for value in values {
        encoder.write_vec3(value)?;
    }
    Ok(())
}

fn write_derived_bounds<E>(
    encoder: &mut StampEncoder<'_, E>,
    derived: &Derived<Aabb>,
) -> ControlledStampResult<(), E> {
    if let Some(value) = derived.value() {
        encoder.write_tag(1)?;
        encoder.write_aabb(*value)
    } else {
        encoder.write_tag(0)?;
        encoder.write_tag(match derived.reason() {
            Some(DerivedAbsenceReason::NotProvided) => 0,
            Some(DerivedAbsenceReason::Invalidated) => 1,
            Some(DerivedAbsenceReason::Unsupported) => 2,
            Some(DerivedAbsenceReason::OmittedByPolicy) => 3,
            None => 4,
        })
    }
}

fn write_morph_bounds_policy<E>(
    encoder: &mut StampEncoder<'_, E>,
    policy: MorphBoundsPolicy,
) -> ControlledStampResult<(), E> {
    encoder.write_tag(match policy {
        MorphBoundsPolicy::BaseGeometry => 0,
        MorphBoundsPolicy::DefaultWeights => 1,
    })
}

fn write_optional_u32<E>(
    encoder: &mut StampEncoder<'_, E>,
    value: Option<u32>,
) -> ControlledStampResult<(), E> {
    if let Some(value) = value {
        encoder.write_tag(1)?;
        encoder.write_u32(value)
    } else {
        encoder.write_tag(0)
    }
}

fn write_authored_transform<E>(
    encoder: &mut StampEncoder<'_, E>,
    transform: AuthoredTransform,
) -> ControlledStampResult<(), E> {
    match transform.kind() {
        AuthoredTransformKind::Matrix => {
            encoder.write_tag(0)?;
            write_mat4(
                encoder,
                transform
                    .authored_matrix()
                    .ok_or(DerivedInputStampError::InvalidAuthoredTransform)?,
            )
        }
        AuthoredTransformKind::Trs => {
            encoder.write_tag(1)?;
            let trs = transform
                .authored_trs()
                .ok_or(DerivedInputStampError::InvalidAuthoredTransform)?;
            encoder.write_vec3(trs.translation())?;
            let rotation = trs.rotation();
            encoder.write_f32(rotation.x)?;
            encoder.write_f32(rotation.y)?;
            encoder.write_f32(rotation.z)?;
            encoder.write_f32(rotation.w)?;
            encoder.write_vec3(trs.scale())
        }
    }
}

fn write_mat4<E>(encoder: &mut StampEncoder<'_, E>, matrix: Mat4) -> ControlledStampResult<(), E> {
    for column in matrix.cols {
        for component in column {
            encoder.write_f32(component)?;
        }
    }
    Ok(())
}

fn write_influence_rows<E>(
    encoder: &mut StampEncoder<'_, E>,
    rows: Option<&InfluenceRows>,
) -> ControlledStampResult<(), E> {
    let Some(rows) = rows else {
        return encoder.write_tag(0);
    };
    encoder.write_tag(1)?;
    encoder.write_len(rows.vertex_offsets().len())?;
    for offset in rows.vertex_offsets() {
        encoder.write_u32(*offset)?;
    }
    encoder.write_len(rows.influences().len())?;
    for influence in rows.influences() {
        encoder.write_u32(influence.palette_joint())?;
        encoder.write_f32(influence.weight())?;
        encoder.write_u32(influence.source_order())?;
    }
    Ok(())
}

fn write_skin<E>(encoder: &mut StampEncoder<'_, E>, skin: &Skin) -> ControlledStampResult<(), E> {
    encoder.write_len(skin.joints().len())?;
    for joint in skin.joints() {
        encoder.write_u32(joint.as_u32())?;
    }
    encoder.write_len(skin.inverse_bind_matrices().len())?;
    for inverse_bind in skin.inverse_bind_matrices() {
        write_mat4(encoder, *inverse_bind)?;
    }
    write_optional_u32(encoder, skin.skeleton_root_hint().map(|root| root.as_u32()))?;
    write_skin_matrix_spaces(encoder, skin.matrix_spaces())
}

fn write_skin_matrix_spaces<E>(
    encoder: &mut StampEncoder<'_, E>,
    spaces: SkinMatrixSpaces,
) -> ControlledStampResult<(), E> {
    write_skin_matrix_space(encoder, spaces.input_position())?;
    write_skin_matrix_space(encoder, spaces.inverse_bind_from())?;
    write_skin_matrix_space(encoder, spaces.inverse_bind_to())?;
    write_skin_matrix_space(encoder, spaces.current_joint_from())?;
    write_skin_matrix_space(encoder, spaces.current_joint_to())?;
    write_skin_matrix_space(encoder, spaces.output_position())?;
    if let Some(correction) = spaces.correction() {
        encoder.write_tag(1)?;
        write_skin_matrix_space(encoder, correction.from())?;
        write_skin_matrix_space(encoder, correction.to())?;
        write_mat4(encoder, correction.matrix())?;
    } else {
        encoder.write_tag(0)?;
    }
    Ok(())
}

fn write_skin_matrix_space<E>(
    encoder: &mut StampEncoder<'_, E>,
    space: SkinMatrixSpace,
) -> ControlledStampResult<(), E> {
    encoder.write_tag(match space {
        SkinMatrixSpace::LogicalMeshLocalBind => 0,
        SkinMatrixSpace::JointLocalBind => 1,
        SkinMatrixSpace::JointLocal => 2,
        SkinMatrixSpace::Scene => 3,
        SkinMatrixSpace::MeshInstanceLocal => 4,
    })
}

fn write_scene_space<E>(
    encoder: &mut StampEncoder<'_, E>,
    scene_space: &SceneSpace,
) -> ControlledStampResult<(), E> {
    encoder.write_tag(match scene_space.handedness {
        Handedness::Unknown => 0,
        Handedness::Right => 1,
        Handedness::Left => 2,
    })?;
    write_axis(encoder, scene_space.up_axis)?;
    write_axis(encoder, scene_space.front_axis)?;
    if let Some(scale) = scene_space.unit_scale_to_meters {
        encoder.write_tag(1)?;
        encoder.write_f64(scale)?;
    } else {
        encoder.write_tag(0)?;
    }
    encoder.write_tag(match scene_space.front_face_winding {
        FrontFaceWinding::Unknown => 0,
        FrontFaceWinding::Clockwise => 1,
        FrontFaceWinding::CounterClockwise => 2,
    })?;
    encoder.write_tag(match scene_space.uv_origin {
        UvOrigin::Unknown => 0,
        UvOrigin::UpperLeft => 1,
        UvOrigin::LowerLeft => 2,
    })?;
    encoder.write_tag(match scene_space.transform_convention {
        TransformConvention::Unknown => 0,
        TransformConvention::ColumnVectorLocalToParent => 1,
    })?;
    match &scene_space.tangent_basis {
        TangentBasisConvention::Unknown => encoder.write_tag(0)?,
        TangentBasisConvention::MikkTspace => encoder.write_tag(1)?,
        TangentBasisConvention::Namespaced { schema_id } => {
            encoder.write_tag(2)?;
            write_schema_id(encoder, schema_id)?;
        }
    }
    match &scene_space.tangent_handedness_encoding {
        TangentHandednessEncoding::Unknown => encoder.write_tag(0)?,
        TangentHandednessEncoding::SignInW => encoder.write_tag(1)?,
        TangentHandednessEncoding::Namespaced { schema_id } => {
            encoder.write_tag(2)?;
            write_schema_id(encoder, schema_id)?;
        }
    }
    Ok(())
}

fn write_axis<E>(encoder: &mut StampEncoder<'_, E>, axis: Axis) -> ControlledStampResult<(), E> {
    encoder.write_tag(match axis {
        Axis::Unknown => 0,
        Axis::PositiveX => 1,
        Axis::NegativeX => 2,
        Axis::PositiveY => 3,
        Axis::NegativeY => 4,
        Axis::PositiveZ => 5,
        Axis::NegativeZ => 6,
    })
}

fn write_schema_id<E>(
    encoder: &mut StampEncoder<'_, E>,
    schema_id: &ExtensionSchemaId,
) -> ControlledStampResult<(), E> {
    encoder.write_bytes(schema_id.name().as_str().as_bytes())?;
    encoder.write_u32(schema_id.version().get())
}
