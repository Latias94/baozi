use std::num::NonZeroU32;

use baozi_operation::{CanonicalRecordView, PublishedCanonicalStore, RecordBlock};

use crate::canonical_scene::store::schema::spatial::{
    SPATIAL_AXIS_NEGATIVE_X, SPATIAL_AXIS_NEGATIVE_Y, SPATIAL_AXIS_NEGATIVE_Z,
    SPATIAL_AXIS_POSITIVE_X, SPATIAL_AXIS_POSITIVE_Y, SPATIAL_AXIS_POSITIVE_Z,
    SPATIAL_AXIS_UNKNOWN, SPATIAL_CAMERA_FIELD_COUNT, SPATIAL_CAMERA_ORTHOGRAPHIC,
    SPATIAL_CAMERA_PERSPECTIVE, SPATIAL_DIRECTORY_FIELD_COUNT, SPATIAL_HANDEDNESS_LEFT,
    SPATIAL_HANDEDNESS_RIGHT, SPATIAL_HANDEDNESS_UNKNOWN, SPATIAL_LIGHT_DIRECTIONAL,
    SPATIAL_LIGHT_FIELD_COUNT, SPATIAL_LIGHT_POINT, SPATIAL_LIGHT_SPOT,
    SPATIAL_TANGENT_BASIS_MIKKTSPACE, SPATIAL_TANGENT_BASIS_NAMESPACED,
    SPATIAL_TANGENT_BASIS_UNKNOWN, SPATIAL_TANGENT_ENCODING_NAMESPACED,
    SPATIAL_TANGENT_ENCODING_SIGN_IN_W, SPATIAL_TANGENT_ENCODING_UNKNOWN,
    SPATIAL_TRANSFORM_COLUMN_VECTOR_LOCAL_TO_PARENT, SPATIAL_TRANSFORM_UNKNOWN,
    SPATIAL_UV_ORIGIN_LOWER_LEFT, SPATIAL_UV_ORIGIN_UNKNOWN, SPATIAL_UV_ORIGIN_UPPER_LEFT,
    SPATIAL_WINDING_CLOCKWISE, SPATIAL_WINDING_COUNTER_CLOCKWISE, SPATIAL_WINDING_UNKNOWN,
    spatial_camera_field, spatial_directory_field, spatial_light_field,
};
use crate::canonical_scene::store::schema::{
    PRESENCE_ABSENT, PRESENCE_PRESENT, SceneDirectorySchema, node_scene_node_field,
};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::entity::{CameraProjection, LightKind};
use crate::math::Vec3;
use crate::scene_space::{Axis, FrontFaceWinding, Handedness, TransformConvention, UvOrigin};

use super::{
    NodeSceneSemanticResult, invariant, read_node_spatial_attachment, read_scalar,
    read_u32_as_usize,
};

type SceneCanonicalStore = PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>;

pub(super) struct NodeSceneSpatialTables<'scene> {
    directory: CanonicalRecordView<'scene>,
    cameras: Option<CanonicalRecordView<'scene>>,
    lights: Option<CanonicalRecordView<'scene>>,
    tangent_basis_schema_name: Option<&'scene str>,
    tangent_encoding_schema_name: Option<&'scene str>,
}

impl<'scene> NodeSceneSpatialTables<'scene> {
    pub(super) fn try_new(
        canonical: &'scene SceneCanonicalStore,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        let directory = canonical
            .records(proof.spatial_directory().into_store())
            .ok_or_else(|| invariant("validated spatial directory is unavailable or mistyped"))?;
        if directory.rows() != 1 || directory.fields() != SPATIAL_DIRECTORY_FIELD_COUNT {
            return Err(invariant(
                "validated spatial directory layout disagrees with validation proof",
            ));
        }
        let tangent_basis_schema_name = resolve_schema_name(
            canonical,
            proof.tangent_basis_schema_name(),
            "validated tangent-basis schema name is unavailable or mistyped",
        )?;
        let tangent_encoding_schema_name = resolve_schema_name(
            canonical,
            proof.tangent_encoding_schema_name(),
            "validated tangent-encoding schema name is unavailable or mistyped",
        )?;
        Ok(Self {
            directory,
            cameras: resolve_table(
                canonical,
                proof.cameras(),
                proof.camera_count(),
                SPATIAL_CAMERA_FIELD_COUNT,
                "validated camera table is unavailable or mistyped",
                "validated camera table presence disagrees with validation proof",
                "validated camera table layout disagrees with validation proof",
            )?,
            lights: resolve_table(
                canonical,
                proof.lights(),
                proof.light_count(),
                SPATIAL_LIGHT_FIELD_COUNT,
                "validated light table is unavailable or mistyped",
                "validated light table presence disagrees with validation proof",
                "validated light table layout disagrees with validation proof",
            )?,
            tangent_basis_schema_name,
            tangent_encoding_schema_name,
        })
    }

    pub(super) fn scene_space(
        &self,
    ) -> NodeSceneSemanticResult<NodeSceneSpatialSceneSpace<'scene>> {
        let tangent_basis = decode_tangent_basis(
            read_scalar::<u8>(
                self.directory,
                0,
                spatial_directory_field::TANGENT_BASIS,
                "validated tangent-basis code is unreadable",
            )?,
            self.tangent_basis_schema(
                spatial_directory_field::TANGENT_BASIS_SCHEMA_VERSION,
                "validated tangent-basis schema version is unreadable",
            )?,
        )?;
        let tangent_encoding = decode_tangent_encoding(
            read_scalar::<u8>(
                self.directory,
                0,
                spatial_directory_field::TANGENT_ENCODING,
                "validated tangent-encoding code is unreadable",
            )?,
            self.tangent_encoding_schema(
                spatial_directory_field::TANGENT_ENCODING_SCHEMA_VERSION,
                "validated tangent-encoding schema version is unreadable",
            )?,
        )?;
        Ok(NodeSceneSpatialSceneSpace {
            handedness: decode_handedness(read_scalar::<u8>(
                self.directory,
                0,
                spatial_directory_field::HANDEDNESS,
                "validated handedness code is unreadable",
            )?)?,
            up_axis: decode_axis(read_scalar::<u8>(
                self.directory,
                0,
                spatial_directory_field::UP_AXIS,
                "validated up-axis code is unreadable",
            )?)?,
            front_axis: decode_axis(read_scalar::<u8>(
                self.directory,
                0,
                spatial_directory_field::FRONT_AXIS,
                "validated front-axis code is unreadable",
            )?)?,
            unit_scale_to_meters: read_optional_f64(
                self.directory,
                spatial_directory_field::UNIT_SCALE_PRESENCE,
                spatial_directory_field::UNIT_SCALE_TO_METERS,
                "validated unit scale is unreadable",
            )?,
            front_face_winding: decode_winding(read_scalar::<u8>(
                self.directory,
                0,
                spatial_directory_field::FRONT_FACE_WINDING,
                "validated front-face winding code is unreadable",
            )?)?,
            uv_origin: decode_uv_origin(read_scalar::<u8>(
                self.directory,
                0,
                spatial_directory_field::UV_ORIGIN,
                "validated UV-origin code is unreadable",
            )?)?,
            transform_convention: decode_transform_convention(read_scalar::<u8>(
                self.directory,
                0,
                spatial_directory_field::TRANSFORM_CONVENTION,
                "validated transform-convention code is unreadable",
            )?)?,
            tangent_basis,
            tangent_encoding,
        })
    }

    pub(super) fn camera_at(
        &self,
        source_index: usize,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSpatialCamera>> {
        let Some(table) = self.cameras else {
            return Ok(None);
        };
        if source_index >= table.rows() {
            return Ok(None);
        }
        if read_u32_as_usize(
            table,
            source_index,
            spatial_camera_field::CAMERA,
            "validated camera identity is unreadable",
        )? != source_index
        {
            return Err(invariant("validated camera identity is not dense"));
        }
        let primary = read_scalar::<f32>(
            table,
            source_index,
            spatial_camera_field::PRIMARY,
            "validated camera primary parameter is unreadable",
        )?;
        let secondary = read_optional_f32(
            table,
            source_index,
            spatial_camera_field::SECONDARY_PRESENCE,
            spatial_camera_field::SECONDARY,
            "validated camera secondary parameter is unreadable",
        )?;
        let near = read_scalar::<f32>(
            table,
            source_index,
            spatial_camera_field::NEAR,
            "validated camera near plane is unreadable",
        )?;
        let far = read_optional_f32(
            table,
            source_index,
            spatial_camera_field::FAR_PRESENCE,
            spatial_camera_field::FAR,
            "validated camera far plane is unreadable",
        )?;
        let projection = match read_scalar::<u8>(
            table,
            source_index,
            spatial_camera_field::PROJECTION,
            "validated camera projection code is unreadable",
        )? {
            SPATIAL_CAMERA_PERSPECTIVE => {
                CameraProjection::perspective(primary, secondary, near, far)
            }
            SPATIAL_CAMERA_ORTHOGRAPHIC => CameraProjection::orthographic(
                primary,
                secondary.ok_or_else(|| {
                    invariant("validated orthographic camera is missing vertical magnification")
                })?,
                near,
                far.ok_or_else(|| invariant("validated orthographic camera is missing far plane"))?,
            ),
            _ => return Err(invariant("validated camera projection code is unknown")),
        }
        .map_err(|_| invariant("validated camera projection is not canonical"))?;
        Ok(Some(NodeSceneSpatialCamera {
            source_index,
            projection,
        }))
    }

    pub(super) fn light_at(
        &self,
        source_index: usize,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSpatialLight>> {
        let Some(table) = self.lights else {
            return Ok(None);
        };
        if source_index >= table.rows() {
            return Ok(None);
        }
        if read_u32_as_usize(
            table,
            source_index,
            spatial_light_field::LIGHT,
            "validated light identity is unreadable",
        )? != source_index
        {
            return Err(invariant("validated light identity is not dense"));
        }
        let range = read_optional_f32(
            table,
            source_index,
            spatial_light_field::RANGE_PRESENCE,
            spatial_light_field::RANGE,
            "validated light range is unreadable",
        )?;
        let inner = read_optional_f32(
            table,
            source_index,
            spatial_light_field::INNER_CONE_PRESENCE,
            spatial_light_field::INNER_CONE,
            "validated light inner cone is unreadable",
        )?;
        let outer = read_optional_f32(
            table,
            source_index,
            spatial_light_field::OUTER_CONE_PRESENCE,
            spatial_light_field::OUTER_CONE,
            "validated light outer cone is unreadable",
        )?;
        let kind = match read_scalar::<u8>(
            table,
            source_index,
            spatial_light_field::KIND,
            "validated light-kind code is unreadable",
        )? {
            SPATIAL_LIGHT_DIRECTIONAL if range.is_none() && inner.is_none() && outer.is_none() => {
                LightKind::directional()
            }
            SPATIAL_LIGHT_POINT if inner.is_none() && outer.is_none() => LightKind::point(range)
                .map_err(|_| invariant("validated point-light kind is not canonical"))?,
            SPATIAL_LIGHT_SPOT => LightKind::spot(
                range,
                inner.ok_or_else(|| invariant("validated spot light is missing inner cone"))?,
                outer.ok_or_else(|| invariant("validated spot light is missing outer cone"))?,
            )
            .map_err(|_| invariant("validated spot-light kind is not canonical"))?,
            _ => return Err(invariant("validated light-kind fields are inconsistent")),
        };
        Ok(Some(NodeSceneSpatialLight {
            source_index,
            kind,
            tint: Vec3::new(
                read_scalar::<f32>(
                    table,
                    source_index,
                    spatial_light_field::TINT_RED,
                    "validated light red tint is unreadable",
                )?,
                read_scalar::<f32>(
                    table,
                    source_index,
                    spatial_light_field::TINT_GREEN,
                    "validated light green tint is unreadable",
                )?,
                read_scalar::<f32>(
                    table,
                    source_index,
                    spatial_light_field::TINT_BLUE,
                    "validated light blue tint is unreadable",
                )?,
            ),
            intensity: read_scalar::<f32>(
                table,
                source_index,
                spatial_light_field::INTENSITY,
                "validated light intensity is unreadable",
            )?,
        }))
    }

    fn tangent_basis_schema(
        &self,
        version_field: usize,
        reason: &'static str,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSpatialSchemaRef<'scene>>> {
        schema_ref(
            self.tangent_basis_schema_name,
            self.directory,
            version_field,
            reason,
        )
    }

    fn tangent_encoding_schema(
        &self,
        version_field: usize,
        reason: &'static str,
    ) -> NodeSceneSemanticResult<Option<NodeSceneSpatialSchemaRef<'scene>>> {
        schema_ref(
            self.tangent_encoding_schema_name,
            self.directory,
            version_field,
            reason,
        )
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
    transform_convention: TransformConvention,
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
    pub(in crate::canonical_scene::store) const fn transform_convention(
        self,
    ) -> TransformConvention {
        self.transform_convention
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
    nodes: CanonicalRecordView<'scene>,
    next_source_node: usize,
    source_node_count: usize,
}

impl<'scene> NodeSceneSpatialPlacementsCursor<'scene> {
    pub(super) const fn new(
        nodes: CanonicalRecordView<'scene>,
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
            let row = source_node
                .checked_add(1)
                .ok_or_else(|| invariant("spatial placement node row overflows"))?;
            let camera = read_node_spatial_attachment(
                self.nodes,
                row,
                node_scene_node_field::CAMERA_PRESENCE,
                node_scene_node_field::CAMERA,
                "validated camera attachment is unreadable",
            )?;
            let light = read_node_spatial_attachment(
                self.nodes,
                row,
                node_scene_node_field::LIGHT_PRESENCE,
                node_scene_node_field::LIGHT,
                "validated light attachment is unreadable",
            )?;
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
        nodes: CanonicalRecordView<'scene>,
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
        nodes: CanonicalRecordView<'scene>,
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

fn resolve_table<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<crate::canonical_scene::store::schema::CanonicalBlockRef<Tag>>,
    expected_rows: usize,
    expected_fields: usize,
    unavailable_reason: &'static str,
    presence_reason: &'static str,
    layout_reason: &'static str,
) -> NodeSceneSemanticResult<Option<CanonicalRecordView<'scene>>>
where
    Tag: crate::canonical_scene::store::schema::CanonicalSchema<StoreKind = RecordBlock>,
{
    match (expected_rows, reference) {
        (0, None) => Ok(None),
        (0, Some(_)) | (_, None) => Err(invariant(presence_reason)),
        (_, Some(reference)) => {
            let table = canonical
                .records(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))?;
            if table.rows() != expected_rows || table.fields() != expected_fields {
                return Err(invariant(layout_reason));
            }
            Ok(Some(table))
        }
    }
}

fn resolve_schema_name<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<crate::canonical_scene::store::schema::CanonicalBlockRef<Tag>>,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene str>>
where
    Tag: crate::canonical_scene::store::schema::CanonicalSchema<
            StoreKind = baozi_operation::Utf8Block,
        >,
{
    reference
        .map(|reference| {
            canonical
                .utf8(reference.into_store())
                .filter(|name| !name.is_empty())
                .ok_or_else(|| invariant(reason))
        })
        .transpose()
}

fn schema_ref<'scene>(
    name: Option<&'scene str>,
    directory: CanonicalRecordView<'_>,
    version_field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<NodeSceneSpatialSchemaRef<'scene>>> {
    name.map(|name| {
        let version = NonZeroU32::new(read_scalar::<u32>(directory, 0, version_field, reason)?)
            .ok_or_else(|| invariant(reason))?;
        Ok(NodeSceneSpatialSchemaRef { name, version })
    })
    .transpose()
}

fn read_optional_f32(
    table: CanonicalRecordView<'_>,
    row: usize,
    presence_field: usize,
    value_field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<f32>> {
    match read_scalar::<u8>(table, row, presence_field, reason)? {
        PRESENCE_ABSENT => Ok(None),
        PRESENCE_PRESENT => Ok(Some(read_scalar::<f32>(table, row, value_field, reason)?)),
        _ => Err(invariant(reason)),
    }
}

fn read_optional_f64(
    table: CanonicalRecordView<'_>,
    presence_field: usize,
    value_field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<f64>> {
    match read_scalar::<u8>(table, 0, presence_field, reason)? {
        PRESENCE_ABSENT => Ok(None),
        PRESENCE_PRESENT => Ok(Some(read_scalar::<f64>(table, 0, value_field, reason)?)),
        _ => Err(invariant(reason)),
    }
}

fn decode_handedness(value: u8) -> NodeSceneSemanticResult<Handedness> {
    match value {
        SPATIAL_HANDEDNESS_UNKNOWN => Ok(Handedness::Unknown),
        SPATIAL_HANDEDNESS_RIGHT => Ok(Handedness::Right),
        SPATIAL_HANDEDNESS_LEFT => Ok(Handedness::Left),
        _ => Err(invariant("validated handedness code is unknown")),
    }
}

fn decode_axis(value: u8) -> NodeSceneSemanticResult<Axis> {
    match value {
        SPATIAL_AXIS_UNKNOWN => Ok(Axis::Unknown),
        SPATIAL_AXIS_POSITIVE_X => Ok(Axis::PositiveX),
        SPATIAL_AXIS_NEGATIVE_X => Ok(Axis::NegativeX),
        SPATIAL_AXIS_POSITIVE_Y => Ok(Axis::PositiveY),
        SPATIAL_AXIS_NEGATIVE_Y => Ok(Axis::NegativeY),
        SPATIAL_AXIS_POSITIVE_Z => Ok(Axis::PositiveZ),
        SPATIAL_AXIS_NEGATIVE_Z => Ok(Axis::NegativeZ),
        _ => Err(invariant("validated axis code is unknown")),
    }
}

fn decode_winding(value: u8) -> NodeSceneSemanticResult<FrontFaceWinding> {
    match value {
        SPATIAL_WINDING_UNKNOWN => Ok(FrontFaceWinding::Unknown),
        SPATIAL_WINDING_CLOCKWISE => Ok(FrontFaceWinding::Clockwise),
        SPATIAL_WINDING_COUNTER_CLOCKWISE => Ok(FrontFaceWinding::CounterClockwise),
        _ => Err(invariant("validated front-face winding code is unknown")),
    }
}

fn decode_uv_origin(value: u8) -> NodeSceneSemanticResult<UvOrigin> {
    match value {
        SPATIAL_UV_ORIGIN_UNKNOWN => Ok(UvOrigin::Unknown),
        SPATIAL_UV_ORIGIN_UPPER_LEFT => Ok(UvOrigin::UpperLeft),
        SPATIAL_UV_ORIGIN_LOWER_LEFT => Ok(UvOrigin::LowerLeft),
        _ => Err(invariant("validated UV-origin code is unknown")),
    }
}

fn decode_transform_convention(value: u8) -> NodeSceneSemanticResult<TransformConvention> {
    match value {
        SPATIAL_TRANSFORM_UNKNOWN => Ok(TransformConvention::Unknown),
        SPATIAL_TRANSFORM_COLUMN_VECTOR_LOCAL_TO_PARENT => {
            Ok(TransformConvention::ColumnVectorLocalToParent)
        }
        _ => Err(invariant("validated transform-convention code is unknown")),
    }
}

fn decode_tangent_basis<'scene>(
    value: u8,
    schema: Option<NodeSceneSpatialSchemaRef<'scene>>,
) -> NodeSceneSemanticResult<NodeSceneSpatialTangentBasis<'scene>> {
    match (value, schema) {
        (SPATIAL_TANGENT_BASIS_UNKNOWN, None) => Ok(NodeSceneSpatialTangentBasis::Unknown),
        (SPATIAL_TANGENT_BASIS_MIKKTSPACE, None) => Ok(NodeSceneSpatialTangentBasis::MikkTspace),
        (SPATIAL_TANGENT_BASIS_NAMESPACED, Some(schema)) => {
            Ok(NodeSceneSpatialTangentBasis::Namespaced(schema))
        }
        _ => Err(invariant("validated tangent-basis fields are inconsistent")),
    }
}

fn decode_tangent_encoding<'scene>(
    value: u8,
    schema: Option<NodeSceneSpatialSchemaRef<'scene>>,
) -> NodeSceneSemanticResult<NodeSceneSpatialTangentEncoding<'scene>> {
    match (value, schema) {
        (SPATIAL_TANGENT_ENCODING_UNKNOWN, None) => Ok(NodeSceneSpatialTangentEncoding::Unknown),
        (SPATIAL_TANGENT_ENCODING_SIGN_IN_W, None) => Ok(NodeSceneSpatialTangentEncoding::SignInW),
        (SPATIAL_TANGENT_ENCODING_NAMESPACED, Some(schema)) => {
            Ok(NodeSceneSpatialTangentEncoding::Namespaced(schema))
        }
        _ => Err(invariant(
            "validated tangent-encoding fields are inconsistent",
        )),
    }
}
