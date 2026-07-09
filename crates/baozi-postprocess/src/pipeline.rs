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

    pub fn name(self) -> &'static str {
        match self {
            Self::ValidateScene => "ValidateScene",
            Self::ApplyGlobalScale => "ApplyGlobalScale",
            Self::NormalizeCoordinates => "NormalizeCoordinates",
            Self::Triangulate => "Triangulate",
            Self::SortByPrimitiveType => "SortByPrimitiveType",
            Self::FindDegenerates => "FindDegenerates",
            Self::FindInvalidData => "FindInvalidData",
            Self::JoinIdenticalVertices => "JoinIdenticalVertices",
            Self::GenerateNormals => "GenerateNormals",
            Self::GenerateTangents => "GenerateTangents",
            Self::GenerateBoundingBoxes => "GenerateBoundingBoxes",
            Self::OptimizeMeshes => "OptimizeMeshes",
            Self::OptimizeGraph => "OptimizeGraph",
        }
    }

    pub fn is_implemented(self) -> bool {
        matches!(
            self,
            Self::ValidateScene
                | Self::Triangulate
                | Self::GenerateNormals
                | Self::GenerateBoundingBoxes
        )
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
                PostProcessStep::GenerateNormals => generate_normals(&mut scene)?,
                PostProcessStep::GenerateBoundingBoxes => generate_bounding_boxes(&mut scene)?,
                step => {
                    return Err(postprocess_error(
                        step.name(),
                        "postprocess step is not implemented yet",
                    ));
                }
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

fn generate_normals(scene: &mut Scene) -> Result<()> {
    validate_scene(scene)?;

    for (mesh_index, mesh) in scene.meshes.iter_mut().enumerate() {
        if !mesh.normals.is_empty() {
            continue;
        }
        if mesh.topology != PrimitiveTopology::Triangles {
            return Err(postprocess_error(
                "GenerateNormals",
                format!(
                    "mesh {mesh_index} has {:?} topology; triangulate before generating normals",
                    mesh.topology
                ),
            ));
        }

        let mut normals = vec![None; mesh.positions.len()];
        if mesh.indices.is_empty() {
            for (triangle_index, triangle) in mesh.positions.chunks_exact(3).enumerate() {
                let normal = triangle_normal(
                    mesh_index,
                    triangle_index,
                    triangle[0],
                    triangle[1],
                    triangle[2],
                )?;
                for vertex_offset in 0..3 {
                    let vertex_index = triangle_index * 3 + vertex_offset;
                    assign_normal(&mut normals[vertex_index], normal, mesh_index, vertex_index)?;
                }
            }
        } else {
            for (triangle_index, triangle) in mesh.indices.chunks_exact(3).enumerate() {
                let indices = [
                    triangle[0] as usize,
                    triangle[1] as usize,
                    triangle[2] as usize,
                ];
                let normal = triangle_normal(
                    mesh_index,
                    triangle_index,
                    mesh.positions[indices[0]],
                    mesh.positions[indices[1]],
                    mesh.positions[indices[2]],
                )?;
                for vertex_index in indices {
                    assign_normal(&mut normals[vertex_index], normal, mesh_index, vertex_index)?;
                }
            }
        }

        mesh.normals = normals
            .into_iter()
            .enumerate()
            .map(|(vertex_index, normal)| {
                normal.ok_or_else(|| {
                    postprocess_error(
                        "GenerateNormals",
                        format!(
                            "mesh {mesh_index} vertex {vertex_index} is not referenced by any triangle"
                        ),
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
    }

    validate_scene(scene)
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

fn triangle_normal(
    mesh_index: usize,
    triangle_index: usize,
    a: Vec3,
    b: Vec3,
    c: Vec3,
) -> Result<Vec3> {
    let ab = sub_vec3(b, a);
    let ac = sub_vec3(c, a);
    let normal = cross_vec3(ab, ac);
    let length_squared = dot_vec3(normal, normal);
    if length_squared <= 1.0e-20 || !length_squared.is_finite() {
        return Err(postprocess_error(
            "GenerateNormals",
            format!("mesh {mesh_index} triangle {triangle_index} is degenerate"),
        ));
    }
    Ok(scale_vec3(normal, length_squared.sqrt().recip()))
}

fn assign_normal(
    slot: &mut Option<Vec3>,
    normal: Vec3,
    mesh_index: usize,
    vertex_index: usize,
) -> Result<()> {
    if let Some(existing) = *slot {
        if normals_compatible(existing, normal) {
            return Ok(());
        }
        return Err(postprocess_error(
            "GenerateNormals",
            format!(
                "mesh {mesh_index} shared vertex {vertex_index} needs multiple generated normals"
            ),
        ));
    }
    *slot = Some(normal);
    Ok(())
}

fn normals_compatible(a: Vec3, b: Vec3) -> bool {
    let delta = sub_vec3(a, b);
    dot_vec3(delta, delta) <= 1.0e-10
}

fn sub_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn scale_vec3(value: Vec3, scale: f32) -> Vec3 {
    Vec3::new(value.x * scale, value.y * scale, value.z * scale)
}

fn cross_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn dot_vec3(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
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
    use baozi_core::{Mesh, MeshBinding, Node};

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
    fn unsupported_steps_return_error_instead_of_noop() {
        let scene = baozi_core::SceneBuilder::new().finish().unwrap();
        let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateTangents]);

        let error = pipeline.run(scene).unwrap_err();

        assert_eq!(
            error,
            BaoziError::PostProcess {
                step: "GenerateTangents",
                message: "postprocess step is not implemented yet".to_owned()
            }
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
                    mesh_bindings: vec![MeshBinding::new(mesh)],
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
                    mesh_bindings: vec![MeshBinding::new(mesh)],
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

    #[test]
    fn generate_normals_fills_indexed_triangle_mesh() {
        let scene = scene_with_mesh(Mesh {
            topology: PrimitiveTopology::Triangles,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            indices: vec![0, 1, 2],
            ..Mesh::default()
        });
        let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateNormals]);

        let scene = pipeline.run(scene).unwrap();

        assert_eq!(scene.meshes[0].normals, vec![Vec3::new(0.0, 0.0, 1.0); 3]);
    }

    #[test]
    fn generate_normals_fills_implicit_triangle_mesh() {
        let scene = scene_with_mesh(Mesh {
            topology: PrimitiveTopology::Triangles,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            ..Mesh::default()
        });
        let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateNormals]);

        let scene = pipeline.run(scene).unwrap();

        assert_eq!(scene.meshes[0].normals, vec![Vec3::new(0.0, 0.0, 1.0); 3]);
    }

    #[test]
    fn generate_normals_does_not_overwrite_existing_normals() {
        let existing = vec![Vec3::new(0.0, 1.0, 0.0); 3];
        let scene = scene_with_mesh(Mesh {
            topology: PrimitiveTopology::Triangles,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            normals: existing.clone(),
            indices: vec![0, 1, 2],
            ..Mesh::default()
        });
        let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateNormals]);

        let scene = pipeline.run(scene).unwrap();

        assert_eq!(scene.meshes[0].normals, existing);
    }

    #[test]
    fn triangulate_then_generate_normals_handles_quad_polygon() {
        let scene = scene_with_mesh(Mesh {
            topology: PrimitiveTopology::Polygons,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            indices: vec![0, 1, 2, 3],
            face_vertex_counts: vec![4],
            ..Mesh::default()
        });
        let pipeline = PostProcessPipeline::new([
            PostProcessStep::Triangulate,
            PostProcessStep::GenerateNormals,
        ]);

        let scene = pipeline.run(scene).unwrap();

        assert_eq!(scene.meshes[0].topology, PrimitiveTopology::Triangles);
        assert_eq!(scene.meshes[0].indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(scene.meshes[0].normals, vec![Vec3::new(0.0, 0.0, 1.0); 4]);
    }

    #[test]
    fn generate_normals_rejects_degenerate_triangle() {
        let scene = scene_with_mesh(Mesh {
            topology: PrimitiveTopology::Triangles,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
            ],
            indices: vec![0, 1, 2],
            ..Mesh::default()
        });
        let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateNormals]);

        let error = pipeline.run(scene).unwrap_err();

        assert_eq!(
            error,
            BaoziError::PostProcess {
                step: "GenerateNormals",
                message: "mesh 0 triangle 0 is degenerate".to_owned()
            }
        );
    }

    #[test]
    fn generate_normals_rejects_polygon_mesh_without_triangulate() {
        let scene = scene_with_mesh(Mesh {
            topology: PrimitiveTopology::Polygons,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            indices: vec![0, 1, 2, 3],
            face_vertex_counts: vec![4],
            ..Mesh::default()
        });
        let pipeline = PostProcessPipeline::new([PostProcessStep::GenerateNormals]);

        let error = pipeline.run(scene).unwrap_err();

        assert_eq!(
            error,
            BaoziError::PostProcess {
                step: "GenerateNormals",
                message: "mesh 0 has Polygons topology; triangulate before generating normals"
                    .to_owned()
            }
        );
    }

    fn scene_with_mesh(mesh: Mesh) -> Scene {
        let mut builder = baozi_core::SceneBuilder::new();
        let mesh = builder.add_mesh(mesh);
        builder
            .add_child_node(
                builder.root(),
                Node {
                    mesh_bindings: vec![MeshBinding::new(mesh)],
                    ..Node::default()
                },
            )
            .unwrap();
        builder.finish().unwrap()
    }
}
