#![no_main]

use baozi_core::{
    Material, MaterialId, Mesh, MeshBinding, MeshId, MetadataValue, Node, NodeId,
    PrimitiveTopology, SceneBuilder, SceneReservationKind, SceneValidationControl,
    SceneValidationError, SceneValidationFailure, SceneValidationLimits, SceneValidationStats,
    SceneValidationWork, Vec3, validate_scene_with,
};
use libfuzzer_sys::fuzz_target;
use std::convert::Infallible;

const MAX_INPUT_BYTES: usize = 4_096;
const MAX_MATERIALS: usize = 4;
const MAX_MESHES: usize = 4;
const MAX_TRIANGLES_PER_MESH: usize = 4;
const MAX_ADDITIONAL_NODES: usize = 8;

fuzz_target!(|data: &[u8]| {
    let bounded = &data[..data.len().min(MAX_INPUT_BYTES)];
    let mut cursor = ByteCursor::new(bounded);
    let lane = Lane::from_byte(cursor.byte());
    let corruption = cursor.byte();
    let limit_selector = cursor.byte();
    let limit_value = cursor.byte();
    let Some(mut builder) = build_valid_scene(&mut cursor) else {
        return;
    };
    if lane == Lane::SingleCorruption && apply_corruption(&mut builder, corruption).is_none() {
        return;
    }

    let limits = validation_limits(lane, limit_selector, limit_value);
    let mut control = FuzzValidationControl;
    match validate_scene_with(builder.finish_draft(), limits, &mut control) {
        Ok(success) => {
            assert_ne!(
                lane,
                Lane::SingleCorruption,
                "a guaranteed-invalid SceneDraft passed central validation"
            );
            assert_stats_within_limits(success.stats(), limits);
            match success.try_publish(|()| Ok::<(), Infallible>(())) {
                Ok(_scene) => {}
                Err(error) => match error {},
            }
        }
        Err(SceneValidationFailure::Scene(error)) => match lane {
            Lane::Valid => panic!("a bounded valid SceneDraft failed central validation: {error}"),
            Lane::SingleCorruption => {}
            Lane::TightLimit => assert!(
                matches!(error, SceneValidationError::LimitExceeded { .. }),
                "a tight validation limit produced a non-limit error: {error}"
            ),
        },
        Err(SceneValidationFailure::Control(error)) => match error {},
    }
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    Valid,
    SingleCorruption,
    TightLimit,
}

impl Lane {
    fn from_byte(value: u8) -> Self {
        match value % 3 {
            0 => Self::Valid,
            1 => Self::SingleCorruption,
            _ => Self::TightLimit,
        }
    }
}

fn build_valid_scene(cursor: &mut ByteCursor<'_>) -> Option<SceneBuilder> {
    let mut builder = SceneBuilder::try_new().ok()?;
    let material_count = 1 + usize::from(cursor.byte()) % MAX_MATERIALS;
    let material_count_u32 = u32::try_from(material_count).ok()?;
    for _ in 0..material_count {
        let mut material = Material::default();
        material.name = Some(cursor.safe_name(16)?);
        material.metallic = cursor.unit_f32();
        material.roughness = cursor.unit_f32();
        builder.add_material(material).ok()?;
    }

    let mesh_count = 1 + usize::from(cursor.byte()) % MAX_MESHES;
    let mesh_count_u32 = u32::try_from(mesh_count).ok()?;
    for _ in 0..mesh_count {
        let triangle_count = 1 + usize::from(cursor.byte()) % MAX_TRIANGLES_PER_MESH;
        let vertex_count = triangle_count.checked_mul(3)?;
        let vertex_count_u32 = u32::try_from(vertex_count).ok()?;
        let positions = cursor.try_vec(vertex_count, ByteCursor::vec3)?;
        let normals = if cursor.byte() & 1 == 0 {
            Vec::new()
        } else {
            cursor.try_vec(vertex_count, ByteCursor::vec3)?
        };
        let indices = if cursor.byte() & 1 == 0 {
            Vec::new()
        } else {
            cursor.try_vec(vertex_count, |cursor| {
                u32::from(cursor.byte()) % vertex_count_u32
            })?
        };
        let mesh = Mesh {
            name: Some(cursor.safe_name(16)?),
            topology: PrimitiveTopology::Triangles,
            positions,
            normals,
            indices,
            material: Some(MaterialId::new(
                u32::from(cursor.byte()) % material_count_u32,
            )),
            ..Mesh::default()
        };
        builder.add_mesh(mesh).ok()?;
    }

    let additional_nodes = 1 + usize::from(cursor.byte()) % MAX_ADDITIONAL_NODES;
    for node_index in 0..additional_nodes {
        let current_node_count = u32::try_from(node_index.checked_add(1)?).ok()?;
        let parent = NodeId::new(u32::from(cursor.byte()) % current_node_count);
        let mut node = Node {
            name: Some(cursor.safe_name(16)?),
            ..Node::default()
        };
        node.mesh_bindings.try_reserve_exact(1).ok()?;
        node.mesh_bindings.push(MeshBinding::new(MeshId::new(
            u32::from(cursor.byte()) % mesh_count_u32,
        )));
        builder.add_child_node(parent, node).ok()?;
    }

    builder
        .metadata_mut()
        .try_insert(
            try_owned("fuzz:value")?,
            MetadataValue::String(cursor.safe_name(16)?),
        )
        .ok()?;
    builder.space_mut().unit_scale_to_meters = Some(0.25 + cursor.unit_f32());
    Some(builder)
}

fn apply_corruption(builder: &mut SceneBuilder, selector: u8) -> Option<()> {
    match selector % 8 {
        0 => {
            builder.node_mut(builder.root())?.parent = Some(NodeId::new(u32::MAX));
        }
        1 => {
            let root = builder.root();
            let child = builder.node_mut(root)?.children.first().copied()?;
            let children = &mut builder.node_mut(root)?.children;
            children.try_reserve_exact(1).ok()?;
            children.push(child);
        }
        2 => {
            let mesh = builder.mesh_mut(MeshId::new(0))?;
            mesh.indices.clear();
            mesh.indices.try_reserve_exact(3).ok()?;
            mesh.indices.extend_from_slice(&[0, 1, u32::MAX]);
        }
        3 => {
            let mesh = builder.mesh_mut(MeshId::new(0))?;
            mesh.normals.clear();
            mesh.normals.try_reserve_exact(1).ok()?;
            mesh.normals.push(Vec3::ZERO);
        }
        4 => {
            builder.mesh_mut(MeshId::new(0))?.material = Some(MaterialId::new(u32::MAX));
        }
        5 => {
            builder.mesh_mut(MeshId::new(0))?.positions.first_mut()?.x = f32::NAN;
        }
        6 => {
            let root = builder.root();
            let bindings = &mut builder.node_mut(root)?.mesh_bindings;
            bindings.try_reserve_exact(1).ok()?;
            bindings.push(MeshBinding::new(MeshId::new(u32::MAX)));
        }
        _ => {
            builder
                .metadata_mut()
                .try_insert(try_owned("Invalid:key")?, MetadataValue::Bool(true))
                .ok()?;
        }
    }
    Some(())
}

fn validation_limits(lane: Lane, selector: u8, value: u8) -> SceneValidationLimits {
    let limits = SceneValidationLimits::default()
        .with_max_nodes(16)
        .with_max_edges(32)
        .with_max_depth(16)
        .with_max_entities(64)
        .with_max_references(128)
        .with_max_payload_cells(2_048)
        .with_max_work(8_192)
        .with_max_metadata_entries(32)
        .with_max_metadata_bytes(1_024)
        .with_max_metadata_key_bytes(64)
        .with_max_metadata_string_bytes(64)
        .with_max_canonical_bytes(1024 * 1024)
        .with_max_scratch_bytes(64 * 1024);
    if lane != Lane::TightLimit {
        return limits;
    }

    match selector % 11 {
        0 => limits.with_max_nodes(u64::from(value % 16)),
        1 => limits.with_max_edges(u64::from(value % 32)),
        2 => limits.with_max_depth(u64::from(value % 16)),
        3 => limits.with_max_entities(u64::from(value % 64)),
        4 => limits.with_max_references(u64::from(value % 128)),
        5 => limits.with_max_payload_cells(u64::from(value) * 8),
        6 => limits.with_max_work(u64::from(value) * 32),
        7 => limits.with_max_metadata_entries(u64::from(value % 32)),
        8 => limits.with_max_metadata_bytes(u64::from(value) * 4),
        9 => limits.with_max_canonical_bytes(u64::from(value) * 4_096),
        _ => limits.with_max_scratch_bytes(u64::from(value) * 256),
    }
}

fn assert_stats_within_limits(stats: SceneValidationStats, limits: SceneValidationLimits) {
    let counts = stats.canonical_counts();
    assert!(counts.nodes <= limits.max_nodes());
    assert!(counts.edges <= limits.max_edges());
    assert!(counts.entities <= limits.max_entities());
    assert!(counts.references <= limits.max_references());
    assert!(counts.payload_cells <= limits.max_payload_cells());
    assert!(counts.metadata_entries <= limits.max_metadata_entries());
    assert!(counts.metadata_bytes <= limits.max_metadata_bytes());
    assert!(stats.work_units() <= limits.max_work());
    assert!(stats.maximum_depth() <= limits.max_depth());
    assert!(stats.footprint().accounted_payload_bytes() <= limits.max_canonical_bytes());
    assert!(stats.scratch_peak_bytes() <= limits.max_scratch_bytes());
}

struct FuzzValidationControl;

impl SceneValidationControl for FuzzValidationControl {
    type Error = Infallible;
    type Reservation = ();

    fn poll(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn charge_work(&mut self, _work: SceneValidationWork, _amount: u64) -> Result<(), Self::Error> {
        Ok(())
    }

    fn reserve(
        &mut self,
        _kind: SceneReservationKind,
        _bytes: u64,
    ) -> Result<Self::Reservation, Self::Error> {
        Ok(())
    }

    fn validate_reservation(
        &self,
        _reservation: &Self::Reservation,
        _kind: SceneReservationKind,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resize_reservation(
        &mut self,
        _reservation: &mut Self::Reservation,
        _kind: SceneReservationKind,
        _required_bytes: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn transition_reservation(
        &mut self,
        _reservation: &mut Self::Reservation,
        _from: SceneReservationKind,
        _to: SceneReservationKind,
        _required_bytes: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct ByteCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ByteCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn byte(&mut self) -> u8 {
        let value = self.bytes.get(self.offset).copied().unwrap_or(0);
        self.offset = self.offset.saturating_add(1);
        value
    }

    fn finite_f32(&mut self) -> f32 {
        let value = i16::from_le_bytes([self.byte(), self.byte()]);
        f32::from(value) / 256.0
    }

    fn unit_f32(&mut self) -> f32 {
        f32::from(self.byte()) / 255.0
    }

    fn vec3(&mut self) -> Vec3 {
        Vec3::new(self.finite_f32(), self.finite_f32(), self.finite_f32())
    }

    fn safe_name(&mut self, maximum: usize) -> Option<String> {
        let length = 1 + usize::from(self.byte()) % maximum;
        let mut output = String::new();
        output.try_reserve_exact(length).ok()?;
        for _ in 0..length {
            output.push(char::from(b'a' + self.byte() % 26));
        }
        Some(output)
    }

    fn try_vec<T>(
        &mut self,
        length: usize,
        mut decode: impl FnMut(&mut Self) -> T,
    ) -> Option<Vec<T>> {
        let mut values = Vec::new();
        values.try_reserve_exact(length).ok()?;
        for _ in 0..length {
            values.push(decode(self));
        }
        Some(values)
    }
}

fn try_owned(value: &str) -> Option<String> {
    let mut output = String::new();
    output.try_reserve_exact(value.len()).ok()?;
    output.push_str(value);
    Some(output)
}
