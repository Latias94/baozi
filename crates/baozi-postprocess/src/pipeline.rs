use baozi_core::{Aabb, BaoziError, PrimitiveTopology, Result, Scene, Vec3, validate_scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostProcessStage {
    RawImported,
    ValidatedImported,
    PostProcessed,
    ValidatedOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostProcessStep {
    ValidateScene,
    ApplyGlobalScale,
    NormalizeCoordinates,
    Triangulate,
    SortByPrimitiveType,
    FindDegenerates,
    FindInvalidData,
    JoinIdenticalVertices,
    GenerateNormals,
    GenerateTangents,
    GenerateBoundingBoxes,
    OptimizeMeshes,
    OptimizeGraph,
}

impl PostProcessStep {
    pub const CANONICAL_ORDER: &'static [Self] = &[
        Self::ValidateScene,
        Self::ApplyGlobalScale,
        Self::NormalizeCoordinates,
        Self::Triangulate,
        Self::SortByPrimitiveType,
        Self::FindDegenerates,
        Self::FindInvalidData,
        Self::JoinIdenticalVertices,
        Self::GenerateNormals,
        Self::GenerateTangents,
        Self::GenerateBoundingBoxes,
        Self::OptimizeMeshes,
        Self::OptimizeGraph,
    ];

    pub fn is_destructive(self) -> bool {
        !matches!(self, Self::ValidateScene | Self::GenerateBoundingBoxes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProcessPipeline {
    steps: Vec<PostProcessStep>,
}

impl PostProcessPipeline {
    pub fn new(steps: impl IntoIterator<Item = PostProcessStep>) -> Self {
        let requested: Vec<_> = steps.into_iter().collect();
        let steps = PostProcessStep::CANONICAL_ORDER
            .iter()
            .copied()
            .filter(|step| requested.contains(step))
            .collect();
        Self { steps }
    }

    pub fn steps(&self) -> &[PostProcessStep] {
        &self.steps
    }

    pub fn run(&self, mut scene: Scene) -> Result<Scene> {
        for step in &self.steps {
            match step {
                PostProcessStep::ValidateScene => validate_scene(&scene)?,
                PostProcessStep::Triangulate => triangulate_scene(&mut scene)?,
                PostProcessStep::GenerateBoundingBoxes => generate_bounding_boxes(&mut scene)?,
                _ => {}
            }
        }
        Ok(scene)
    }
}

fn triangulate_scene(scene: &mut Scene) -> Result<()> {
    validate_scene(scene)?;

    for mesh in &mut scene.meshes {
        if mesh.topology != PrimitiveTopology::Polygons {
            continue;
        }

        let source_elements: Vec<u32> = if mesh.indices.is_empty() {
            implicit_indices(mesh.element_count())?
        } else {
            mesh.indices.clone()
        };

        let triangle_index_count = mesh
            .face_vertex_counts
            .iter()
            .try_fold(0usize, |total, count| {
                total.checked_add((*count as usize - 2).checked_mul(3)?)
            })
            .ok_or_else(|| postprocess_error("Triangulate", "triangle index count overflow"))?;
        let mut triangulated = Vec::with_capacity(triangle_index_count);
        let mut cursor = 0usize;

        for count in &mesh.face_vertex_counts {
            let count = *count as usize;
            let face = source_elements
                .get(cursor..cursor + count)
                .ok_or_else(|| postprocess_error("Triangulate", "polygon face range is invalid"))?;
            let anchor = face[0];
            for index in 1..count - 1 {
                triangulated.extend([anchor, face[index], face[index + 1]]);
            }
            cursor += count;
        }

        mesh.topology = PrimitiveTopology::Triangles;
        mesh.indices = triangulated;
        mesh.face_vertex_counts.clear();
    }

    validate_scene(scene)
}

fn implicit_indices(element_count: usize) -> Result<Vec<u32>> {
    if element_count > u32::MAX as usize {
        return Err(postprocess_error(
            "Triangulate",
            "implicit polygon indices exceed u32 range",
        ));
    }
    Ok((0..element_count as u32).collect())
}

fn generate_bounding_boxes(scene: &mut Scene) -> Result<()> {
    validate_scene(scene)?;
    for mesh in &mut scene.meshes {
        if mesh.bounds.is_none() {
            mesh.bounds = compute_bounds(&mesh.positions);
        }
    }
    validate_scene(scene)
}

fn compute_bounds(positions: &[Vec3]) -> Option<Aabb> {
    let first = *positions.first()?;
    let mut min = first;
    let mut max = first;
    for position in positions.iter().copied().skip(1) {
        min.x = min.x.min(position.x);
        min.y = min.y.min(position.y);
        min.z = min.z.min(position.z);
        max.x = max.x.max(position.x);
        max.y = max.y.max(position.y);
        max.z = max.z.max(position.z);
    }
    Some(Aabb { min, max })
}

fn postprocess_error(step: &'static str, message: impl Into<String>) -> BaoziError {
    BaoziError::PostProcess {
        step,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_core::{Mesh, Node};

    #[test]
    fn pipeline_uses_canonical_order() {
        let pipeline = PostProcessPipeline::new([
            PostProcessStep::GenerateNormals,
            PostProcessStep::Triangulate,
            PostProcessStep::ValidateScene,
        ]);
        assert_eq!(
            pipeline.steps(),
            &[
                PostProcessStep::ValidateScene,
                PostProcessStep::Triangulate,
                PostProcessStep::GenerateNormals
            ]
        );
    }

    #[test]
    fn triangulate_converts_polygon_faces_to_triangle_indices() {
        let mut builder = baozi_core::SceneBuilder::new();
        let mesh = builder.add_mesh(Mesh {
            topology: PrimitiveTopology::Polygons,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 1.0, 0.0),
            ],
            indices: vec![0, 1, 2, 3, 1, 4, 5],
            face_vertex_counts: vec![4, 3],
            ..Mesh::default()
        });
        builder
            .add_child_node(
                builder.root(),
                Node {
                    meshes: vec![mesh],
                    ..Node::default()
                },
            )
            .unwrap();
        let scene = builder.finish().unwrap();
        let pipeline = PostProcessPipeline::new([PostProcessStep::Triangulate]);

        let scene = pipeline.run(scene).unwrap();
        let mesh = &scene.meshes[0];

        assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
        assert_eq!(mesh.face_vertex_counts, Vec::<u32>::new());
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3, 1, 4, 5]);
    }

    #[test]
    fn generate_bounding_boxes_fills_missing_bounds() {
        let mut builder = baozi_core::SceneBuilder::new();
        let mesh = builder.add_mesh(Mesh {
            positions: vec![
                Vec3::new(2.0, -1.0, 0.5),
                Vec3::new(-3.0, 4.0, 1.5),
                Vec3::new(0.0, 2.0, -2.0),
            ],
            ..Mesh::default()
        });
        builder
            .add_child_node(
                builder.root(),
                Node {
                    meshes: vec![mesh],
                    ..Node::default()
                },
            )
            .unwrap();
        let scene = builder.finish().unwrap();
        let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateBoundingBoxes]);

        let scene = pipeline.run(scene).unwrap();
        let bounds = scene.meshes[0].bounds.unwrap();

        assert_eq!(bounds.min, Vec3::new(-3.0, -1.0, -2.0));
        assert_eq!(bounds.max, Vec3::new(2.0, 4.0, 1.5));
    }
}
