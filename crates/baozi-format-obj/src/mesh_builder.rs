use crate::ObjFailure;
use crate::mtl::{self, MaterialLibrary, ParsedMaterial, ParsedMetadata, ParsedMetadataValue};
use crate::obj::{Face, FaceVertex, ParsedObj, StateNameId};
use crate::parser::usize_amount;
use baozi_core::{
    Aabb, AlphaMode, ColorSpace, Mat4, Material, MaterialId, Mesh, MeshBinding, MetadataMap,
    PrimitiveTopology, SceneDraft, ShadingModel, TextureRole, TextureSlot, Vec2, Vec3,
};
use baozi_import::{
    ImportStorageClass, ImportWorkClass, MaterialBuildConfig, MeshBuildConfig, MeteredMetadataMap,
    MeteredString, MeteredVec, NodeBuildConfig, ParseResult, ParseScope, PreparedSceneValue,
    SceneValueParts, ScopedSceneBuilder, TextureBuildConfig, TextureSlotBuildConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SegmentKey {
    object: Option<StateNameId>,
    group: Option<StateNameId>,
    material: Option<StateNameId>,
    smoothing: Option<StateNameId>,
}

impl SegmentKey {
    fn from_face(face: &Face<'_>) -> Self {
        Self {
            object: face.object,
            group: face.group,
            material: face.material,
            smoothing: face.smoothing,
        }
    }

    fn display_name(self) -> Option<StateNameId> {
        self.group.or(self.object).or(self.material)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct VertexKey {
    position: usize,
    texcoord: Option<usize>,
    normal: Option<usize>,
}

impl From<FaceVertex> for VertexKey {
    fn from(value: FaceVertex) -> Self {
        Self {
            position: value.position,
            texcoord: value.texcoord,
            normal: value.normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct IndexedVertexKey {
    key: VertexKey,
    occurrence: usize,
}

struct SegmentBuilder<'scope> {
    key: SegmentKey,
    vertex_keys: MeteredVec<'scope, VertexKey>,
    face_vertex_counts: MeteredVec<'scope, u32>,
    has_polygon: bool,
}

fn state_name<'parsed>(
    scope: &ParseScope<'_>,
    parsed: &'parsed ParsedObj<'_>,
    id: StateNameId,
) -> ParseResult<&'parsed str> {
    parsed.state_name(id).ok_or_else(|| {
        scope.format_failure(ObjFailure::internal("segment-state-name-id-out-of-range"))
    })
}

fn charge_segment_strings(
    scope: &ParseScope<'_>,
    parsed: &ParsedObj<'_>,
    key: &SegmentKey,
) -> ParseResult<()> {
    let mut bytes = 0usize;
    if let Some(id) = key.display_name() {
        let name = state_name(scope, parsed, id)?;
        checked_add_string_bytes(scope, &mut bytes, name.len())?;
        checked_add_string_bytes(scope, &mut bytes, name.len())?;
    }
    for (metadata_key, id) in [
        ("obj:object", key.object),
        ("obj:group", key.group),
        ("obj:material", key.material),
        ("obj:smoothing", key.smoothing),
    ] {
        if let Some(id) = id {
            let value = state_name(scope, parsed, id)?;
            checked_add_string_bytes(scope, &mut bytes, metadata_key.len())?;
            checked_add_string_bytes(scope, &mut bytes, value.len())?;
        }
    }

    scope
        .begin_work(
            ImportWorkClass::CanonicalStringBytes,
            usize_amount(scope, bytes, "obj-canonical-string-bytes")?,
        )?
        .complete()
}

fn checked_add_string_bytes(
    scope: &ParseScope<'_>,
    total: &mut usize,
    bytes: usize,
) -> ParseResult<()> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| scope.allocation_failure("obj-canonical-string-counter"))?;
    Ok(())
}

fn owned_canonical_string<'scope>(
    scope: &'scope ParseScope<'_>,
    value: &str,
) -> ParseResult<MeteredString<'scope>> {
    let mut output = scope.metered_string(
        ImportStorageClass::SceneConstruction,
        "obj-canonical-string",
    );
    output.try_push_str(value)?;
    Ok(output)
}

fn canonical_string_parts<'scope>(
    scope: &'scope ParseScope<'_>,
    value: &str,
) -> ParseResult<SceneValueParts<'scope, String>> {
    owned_canonical_string(scope, value)?.try_into_scene_parts()
}

fn optional_canonical_string_parts<'scope>(
    scope: &'scope ParseScope<'_>,
    value: Option<&str>,
) -> ParseResult<SceneValueParts<'scope, Option<String>>> {
    let has_value = value.is_some();
    let mut output = scope.metered_string(
        ImportStorageClass::SceneConstruction,
        "obj-canonical-string",
    );
    if let Some(value) = value {
        output.try_push_str(value)?;
    }
    let output = output.try_into_scene_parts()?;
    Ok(if has_value {
        output.into_some()
    } else {
        output.discard_into_none()
    })
}

fn warn_partial_attributes(
    scope: &ParseScope<'_>,
    has_texcoords: bool,
    missing_texcoords: bool,
    has_normals: bool,
    missing_normals: bool,
) -> ParseResult<()> {
    if has_texcoords && missing_texcoords {
        mtl::push_warning(
            scope,
            None,
            "obj.partial_texcoord_channel",
            format_args!(
                "OBJ segment mixes vertices with and without texture coordinates; the partial texture coordinate channel was omitted"
            ),
        )?;
    }
    if has_normals && missing_normals {
        mtl::push_warning(
            scope,
            None,
            "obj.partial_normal_channel",
            format_args!(
                "OBJ segment mixes vertices with and without normals; the partial normal channel was omitted"
            ),
        )?;
    }
    Ok(())
}

fn insert_string_metadata<'scope>(
    scope: &'scope ParseScope<'_>,
    metadata: &mut MeteredMetadataMap<'scope>,
    key: &'static str,
    value: &str,
) -> ParseResult<()> {
    let parts = canonical_string_parts(scope, key)?
        .zip(canonical_string_parts(scope, value)?)?
        .into_metadata_string_entry();
    metadata.try_insert_parts(parts)
}

impl<'scope> SegmentBuilder<'scope> {
    fn new(scope: &'scope ParseScope<'_>, key: SegmentKey) -> Self {
        Self {
            key,
            vertex_keys: scope.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-segment-vertex-keys",
            ),
            face_vertex_counts: scope.metered_vec(
                ImportStorageClass::SceneConstruction,
                "obj-face-vertex-counts",
            ),
            has_polygon: false,
        }
    }

    fn push_face(&mut self, scope: &'scope ParseScope<'_>, face: &Face<'_>) -> ParseResult<()> {
        let face_vertex_count = face.vertices.len();
        if face_vertex_count < 3 {
            return Err(
                scope.format_failure(ObjFailure::internal("normalized-face-has-too-few-vertices"))
            );
        }
        let face_count_cells = if self.has_polygon {
            1
        } else if face_vertex_count > 3 {
            self.face_vertex_counts
                .len()
                .checked_add(1)
                .ok_or_else(|| scope.allocation_failure("obj-canonical-topology-counter"))?
        } else {
            0
        };
        let canonical_topology_cells = face_vertex_count
            .checked_add(face_count_cells)
            .ok_or_else(|| scope.allocation_failure("obj-canonical-topology-counter"))?;
        let topology_permit = scope.begin_work(
            ImportWorkClass::TopologyCells,
            usize_amount(
                scope,
                canonical_topology_cells,
                "obj-canonical-topology-cells",
            )?,
        )?;

        if face_vertex_count > 3 {
            self.has_polygon = true;
        }
        self.face_vertex_counts.try_push(
            u32::try_from(face_vertex_count)
                .map_err(|_| scope.allocation_failure("obj-face-vertex-count"))?,
        )?;
        self.vertex_keys.try_reserve(face_vertex_count)?;

        for chunk in face.vertices.as_slice().chunks(64) {
            scope.poll()?;
            for vertex in chunk {
                self.vertex_keys.try_push((*vertex).into())?;
            }
        }
        topology_permit.complete()
    }

    fn materialize_vertices(
        &self,
        scope: &'scope ParseScope<'_>,
        parsed: &ParsedObj<'_>,
    ) -> ParseResult<BuiltVertexStreams<'scope>> {
        if self.vertex_keys.is_empty() {
            return Err(
                scope.format_failure(ObjFailure::internal("segment-has-no-vertex-references"))
            );
        }

        let mut sorted_keys = scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-sorted-vertex-keys",
        );
        sorted_keys.try_reserve(self.vertex_keys.len())?;
        for (chunk_index, chunk) in self.vertex_keys.as_slice().chunks(64).enumerate() {
            scope.poll()?;
            let chunk_start = chunk_index
                .checked_mul(64)
                .ok_or_else(|| scope.allocation_failure("obj-vertex-key-copy-offset"))?;
            for (relative, key) in chunk.iter().copied().enumerate() {
                let occurrence = chunk_start
                    .checked_add(relative)
                    .ok_or_else(|| scope.allocation_failure("obj-vertex-key-copy-offset"))?;
                sorted_keys.try_push(IndexedVertexKey { key, occurrence })?;
            }
        }
        mtl::interruptible_sort_by(scope, &mut sorted_keys, |_work, left, right| {
            Ok(left.cmp(right))
        })?;
        let mut first_occurrences = scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-first-vertex-occurrences",
        );
        first_occurrences.try_reserve(self.vertex_keys.len())?;
        mtl::for_work_chunks(
            scope,
            self.vertex_keys.len(),
            "obj-first-vertex-occurrence-init-work",
            |range| {
                first_occurrences.try_resize(range.end, false)?;
                Ok(())
            },
        )?;

        let mut has_texcoords = false;
        let mut has_normals = false;
        let mut missing_texcoords = false;
        let mut missing_normals = false;
        let mut unique_count = 0usize;
        let mut previous_key = None;
        mtl::for_work_chunks(
            scope,
            sorted_keys.len(),
            "obj-vertex-grouping-work",
            |range| {
                for cursor in range {
                    let indexed = sorted_keys.as_slice()[cursor];
                    let is_first = previous_key != Some(indexed.key);
                    previous_key = Some(indexed.key);
                    if !is_first {
                        continue;
                    }
                    first_occurrences.as_mut_slice()[indexed.occurrence] = true;
                    unique_count = unique_count
                        .checked_add(1)
                        .ok_or_else(|| scope.allocation_failure("obj-unique-vertex-count"))?;
                    has_texcoords |= indexed.key.texcoord.is_some();
                    has_normals |= indexed.key.normal.is_some();
                    missing_texcoords |= indexed.key.texcoord.is_none();
                    missing_normals |= indexed.key.normal.is_none();
                }
                Ok(())
            },
        )?;
        warn_partial_attributes(
            scope,
            has_texcoords,
            missing_texcoords,
            has_normals,
            missing_normals,
        )?;
        let keep_texcoords = has_texcoords && !missing_texcoords;
        let keep_normals = has_normals && !missing_normals;

        let vertex_permit = scope.begin_work(
            ImportWorkClass::Vertices,
            usize_amount(scope, unique_count, "obj-canonical-vertices")?,
        )?;
        let channel_count = usize::from(keep_texcoords) + usize::from(keep_normals);
        let attribute_cells = unique_count
            .checked_mul(channel_count)
            .ok_or_else(|| scope.allocation_failure("obj-canonical-attribute-cells"))?;
        let attribute_permit = scope.begin_work(
            ImportWorkClass::AttributeCells,
            usize_amount(scope, attribute_cells, "obj-canonical-attribute-cells")?,
        )?;
        let mut positions = scope.metered_vec(
            ImportStorageClass::SceneConstruction,
            "obj-segment-positions",
        );
        let mut texcoords = scope.metered_vec(
            ImportStorageClass::SceneConstruction,
            "obj-segment-texcoords",
        );
        let mut normals =
            scope.metered_vec(ImportStorageClass::SceneConstruction, "obj-segment-normals");
        let mut indices =
            scope.metered_vec(ImportStorageClass::SceneConstruction, "obj-segment-indices");
        positions.try_reserve(unique_count)?;
        if keep_texcoords {
            texcoords.try_reserve(unique_count)?;
        }
        if keep_normals {
            normals.try_reserve(unique_count)?;
        }
        indices.try_reserve(self.vertex_keys.len())?;
        mtl::for_work_chunks(
            scope,
            self.vertex_keys.len(),
            "obj-vertex-index-init-work",
            |range| {
                indices.try_resize(range.end, 0)?;
                Ok(())
            },
        )?;

        mtl::for_work_chunks(
            scope,
            self.vertex_keys.len(),
            "obj-vertex-materialization-work",
            |range| {
                for occurrence in range {
                    let key = &self.vertex_keys.as_slice()[occurrence];
                    if !first_occurrences.as_slice()[occurrence] {
                        continue;
                    }
                    let canonical_index = u32::try_from(positions.len())
                        .map_err(|_| scope.allocation_failure("obj-vertex-index-space"))?;
                    indices.as_mut_slice()[occurrence] = canonical_index;
                    positions.try_push(parsed.positions.as_slice()[key.position])?;
                    if keep_texcoords {
                        let Some(index) = key.texcoord else {
                            return Err(scope.format_failure(ObjFailure::internal(
                                "complete-texcoord-channel-has-missing-index",
                            )));
                        };
                        texcoords.try_push(parsed.texcoords.as_slice()[index])?;
                    }
                    if keep_normals {
                        let Some(index) = key.normal else {
                            return Err(scope.format_failure(ObjFailure::internal(
                                "complete-normal-channel-has-missing-index",
                            )));
                        };
                        normals.try_push(parsed.normals.as_slice()[index])?;
                    }
                }
                Ok(())
            },
        )?;
        let mut current_key = None;
        let mut canonical_index = 0u32;
        mtl::for_work_chunks(scope, sorted_keys.len(), "obj-vertex-remap-work", |range| {
            for cursor in range {
                let indexed = sorted_keys.as_slice()[cursor];
                if current_key != Some(indexed.key) {
                    current_key = Some(indexed.key);
                    canonical_index = indices.as_slice()[indexed.occurrence];
                } else {
                    indices.as_mut_slice()[indexed.occurrence] = canonical_index;
                }
            }
            Ok(())
        })?;

        attribute_permit.complete()?;
        vertex_permit.complete()?;
        drop(first_occurrences);
        drop(sorted_keys);
        Ok(BuiltVertexStreams {
            positions,
            texcoords,
            normals,
            indices,
        })
    }

    fn finish(
        self,
        scope: &'scope ParseScope<'_>,
        parsed: &ParsedObj<'_>,
        material: Option<MaterialId>,
    ) -> ParseResult<PreparedSceneValue<Mesh>> {
        let streams = self.materialize_vertices(scope, parsed)?;
        let SegmentBuilder {
            key,
            vertex_keys,
            face_vertex_counts,
            has_polygon,
        } = self;
        drop(vertex_keys);
        let BuiltVertexStreams {
            positions,
            texcoords,
            normals,
            indices,
        } = streams;
        let display_name = key
            .display_name()
            .map(|id| state_name(scope, parsed, id))
            .transpose()?;
        let name = optional_canonical_string_parts(scope, display_name)?;
        let topology = if has_polygon {
            PrimitiveTopology::Polygons
        } else {
            PrimitiveTopology::Triangles
        };
        let face_vertex_counts = if has_polygon {
            face_vertex_counts
        } else {
            drop(face_vertex_counts);
            scope.metered_vec(
                ImportStorageClass::SceneConstruction,
                "obj-face-vertex-counts",
            )
        };
        let mut metadata = scope.metered_metadata_map("obj-mesh-metadata-map");
        if let Some(id) = key.object {
            insert_string_metadata(
                scope,
                &mut metadata,
                "obj:object",
                state_name(scope, parsed, id)?,
            )?;
        }
        if let Some(id) = key.group {
            insert_string_metadata(
                scope,
                &mut metadata,
                "obj:group",
                state_name(scope, parsed, id)?,
            )?;
        }
        if let Some(id) = key.material {
            insert_string_metadata(
                scope,
                &mut metadata,
                "obj:material",
                state_name(scope, parsed, id)?,
            )?;
        }
        if let Some(id) = key.smoothing {
            insert_string_metadata(
                scope,
                &mut metadata,
                "obj:smoothing",
                state_name(scope, parsed, id)?,
            )?;
        }
        let mut texcoord_channels = scope.metered_scene_vec("obj-texture-coordinate-channels");
        if !texcoords.is_empty() {
            texcoord_channels.try_push_parts(texcoords.try_into_scene_parts()?)?;
        }
        let bounds = compute_bounds(scope, positions.as_slice())?;

        name.into_mesh(MeshBuildConfig {
            topology,
            material,
            bounds,
        })
        .with_positions(positions.try_into_scene_parts()?)?
        .with_normals(normals.try_into_scene_parts()?)?
        .with_texcoords(texcoord_channels.try_into_scene_parts()?)?
        .with_indices(indices.try_into_scene_parts()?)?
        .with_face_vertex_counts(face_vertex_counts.try_into_scene_parts()?)?
        .with_metadata(metadata.try_into_scene_parts()?)?
        .finish()
    }
}

struct BuiltVertexStreams<'scope> {
    positions: MeteredVec<'scope, Vec3>,
    texcoords: MeteredVec<'scope, Vec2>,
    normals: MeteredVec<'scope, Vec3>,
    indices: MeteredVec<'scope, u32>,
}

pub(crate) fn scene_from_parsed<'scope>(
    scope: &'scope ParseScope<'_>,
    parsed: ParsedObj<'scope>,
    library: &MaterialLibrary<'scope>,
) -> ParseResult<SceneDraft> {
    if parsed.faces.is_empty() {
        return Err(scope.format_failure(ObjFailure::empty_geometry()));
    }

    let mut builder = scope.scene_builder()?;
    let material_bindings = prepare_material_bindings(scope, &mut builder, library, &parsed)?;
    let mut current: Option<SegmentBuilder<'scope>> = None;

    for face in parsed.faces.as_slice() {
        let key = SegmentKey::from_face(face);
        let should_flush = current.as_ref().is_some_and(|segment| segment.key != key);
        if should_flush && let Some(segment) = current.take() {
            flush_segment(scope, &mut builder, &material_bindings, &parsed, segment)?;
        }
        if current.is_none() {
            current = Some(SegmentBuilder::new(scope, key));
        }
        let Some(segment) = current.as_mut() else {
            return Err(scope.format_failure(ObjFailure::internal(
                "mesh-segment-missing-after-initialization",
            )));
        };
        segment.push_face(scope, face)?;
    }

    if let Some(segment) = current {
        flush_segment(scope, &mut builder, &material_bindings, &parsed, segment)?;
    }

    drop(material_bindings);
    builder.finish_draft()
}

fn flush_segment<'scope>(
    scope: &'scope ParseScope<'_>,
    builder: &mut ScopedSceneBuilder<'_, '_>,
    material_bindings: &MaterialBindings<'scope, '_>,
    parsed: &ParsedObj<'_>,
    segment: SegmentBuilder<'scope>,
) -> ParseResult<()> {
    let mesh_permit = scope.begin_work(ImportWorkClass::Meshes, 1)?;
    charge_segment_strings(scope, parsed, &segment.key)?;

    let material = if let Some(id) = segment.key.material {
        let name = state_name(scope, parsed, id)?;
        Some(material_bindings.find(scope, name)?.ok_or_else(|| {
            scope.format_failure(ObjFailure::internal(
                "segment-material-missing-from-prepared-bindings",
            ))
        })?)
    } else {
        None
    };
    let node_name = segment
        .key
        .display_name()
        .map(|id| state_name(scope, parsed, id))
        .transpose()?;
    let prepared_mesh = segment.finish(scope, parsed, material)?;
    let mesh_id = builder.add_prepared_mesh(prepared_mesh)?;
    let prepared_node = optional_canonical_string_parts(scope, node_name)?
        .into_node(NodeBuildConfig {
            transform: Mat4::IDENTITY,
            camera: None,
            light: None,
        })
        .finish()?;
    let root = builder.root();
    let node_id = builder.add_prepared_child_node(root, prepared_node)?;
    builder.add_node_mesh_binding(node_id, MeshBinding::new(mesh_id))?;
    mesh_permit.complete()?;
    Ok(())
}

#[derive(Clone, Copy)]
struct MaterialBinding<'parsed> {
    name: &'parsed str,
    first_use: usize,
    id: Option<MaterialId>,
}

struct MaterialBindings<'scope, 'parsed> {
    values: MeteredVec<'scope, MaterialBinding<'parsed>>,
}

impl MaterialBindings<'_, '_> {
    fn find(&self, scope: &ParseScope<'_>, name: &str) -> ParseResult<Option<MaterialId>> {
        let index =
            mtl::interruptible_binary_search_by(scope, self.values.as_slice(), |work, binding| {
                work.compare_str(binding.name, name)
            })?;
        Ok(index.and_then(|index| self.values.as_slice()[index].id))
    }
}

fn prepare_material_bindings<'scope, 'parsed>(
    scope: &'scope ParseScope<'_>,
    builder: &mut ScopedSceneBuilder<'_, '_>,
    library: &MaterialLibrary<'scope>,
    parsed: &'parsed ParsedObj<'_>,
) -> ParseResult<MaterialBindings<'scope, 'parsed>> {
    let faces = parsed.faces.as_slice();
    let mut values = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-material-bindings",
    );
    mtl::for_work_chunks(
        scope,
        faces.len(),
        "obj-material-binding-scan-work",
        |range| {
            for first_use in range {
                let Some(id) = faces[first_use].material else {
                    continue;
                };
                values.try_push(MaterialBinding {
                    name: state_name(scope, parsed, id)?,
                    first_use,
                    id: None,
                })?;
            }
            Ok(())
        },
    )?;

    mtl::interruptible_sort_by(scope, &mut values, |work, left, right| {
        Ok(work
            .compare_str(left.name, right.name)?
            .then_with(|| left.first_use.cmp(&right.first_use)))
    })?;
    mtl::interruptible_dedup_by(scope, &mut values, |work, previous, current| {
        Ok(work.compare_str(previous.name, current.name)? == std::cmp::Ordering::Equal)
    })?;
    mtl::interruptible_sort_by(scope, &mut values, |_work, left, right| {
        Ok(left.first_use.cmp(&right.first_use))
    })?;

    mtl::for_work_chunks(
        scope,
        values.len(),
        "obj-material-binding-build-work",
        |range| {
            for index in range {
                let name = values.as_slice()[index].name;
                let id = build_material(scope, builder, library, name)?;
                values.as_mut_slice()[index].id = Some(id);
            }
            Ok(())
        },
    )?;
    mtl::interruptible_sort_by(scope, &mut values, |work, left, right| {
        work.compare_str(left.name, right.name)
    })?;
    Ok(MaterialBindings { values })
}

fn build_material<'scope>(
    scope: &'scope ParseScope<'_>,
    builder: &mut ScopedSceneBuilder<'_, '_>,
    library: &MaterialLibrary<'scope>,
    name: &str,
) -> ParseResult<MaterialId> {
    let prepared = if let Some(parsed) = library.find(scope, name)? {
        material_from_parsed(scope, builder, parsed)?
    } else {
        mtl::push_warning(
            scope,
            None,
            "obj.material_missing",
            format_args!("OBJ material '{name}' was referenced but not defined"),
        )?;
        charge_canonical_string_bytes(scope, name.len())?;
        let texture_slots = scope
            .metered_scene_vec::<TextureSlot>("obj-material-texture-slots")
            .try_into_scene_parts()?;
        let metadata = scope
            .metered_metadata_map("obj-material-metadata-map")
            .try_into_scene_parts()?;
        finish_material(
            optional_canonical_string_parts(scope, Some(name))?,
            texture_slots,
            metadata,
            ShadingModel::Unknown,
            baozi_core::Color::WHITE,
            baozi_core::Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
            AlphaMode::Opaque,
        )?
    };
    builder.add_prepared_material(prepared)
}

fn material_from_parsed(
    scope: &ParseScope<'_>,
    builder: &mut ScopedSceneBuilder<'_, '_>,
    parsed: &ParsedMaterial<'_>,
) -> ParseResult<PreparedSceneValue<Material>> {
    charge_material_strings(scope, parsed)?;
    let mut texture_slots = scope.metered_scene_vec("obj-material-texture-slots");

    if let Some(texture) = &parsed.diffuse_texture {
        let texture_value = optional_canonical_string_parts(scope, Some(texture.uri.as_str()))?
            .zip(
                canonical_string_parts(scope, texture.uri.as_str())?.into_external_texture_source(),
            )?
            .into_texture(TextureBuildConfig {
                sampler: Default::default(),
            })
            .finish()?;
        let texture_id = builder.add_prepared_texture(texture_value)?;
        let slot = optional_canonical_string_parts(scope, Some(texture.source_key))?
            .into_texture_slot(TextureSlotBuildConfig {
                texture: texture_id,
                role: TextureRole::Diffuse,
                color_space: ColorSpace::Srgb,
                uv_set: 0,
                scale: 1.0,
                transform: Default::default(),
            });
        texture_slots.try_push_parts(slot)?;
    }

    let alpha_mode = if parsed.base_color.a < 1.0 {
        AlphaMode::Blend
    } else {
        parsed.alpha_mode
    };
    finish_material(
        optional_canonical_string_parts(scope, Some(parsed.name.as_str()))?,
        texture_slots.try_into_scene_parts()?,
        clone_metadata(scope, &parsed.metadata)?,
        ShadingModel::Phong,
        parsed.base_color,
        parsed.emissive,
        alpha_mode,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_material<'scope>(
    name: SceneValueParts<'scope, Option<String>>,
    texture_slots: SceneValueParts<'scope, Vec<TextureSlot>>,
    metadata: SceneValueParts<'scope, MetadataMap>,
    shading_model: ShadingModel,
    base_color: baozi_core::Color,
    emissive: baozi_core::Color,
    alpha_mode: AlphaMode,
) -> ParseResult<PreparedSceneValue<Material>> {
    name.into_material(MaterialBuildConfig {
        shading_model,
        base_color,
        metallic: 0.0,
        roughness: 1.0,
        emissive,
        alpha_mode,
        alpha_cutoff: 0.5,
        double_sided: false,
    })
    .with_texture_slots(texture_slots)?
    .with_metadata(metadata)?
    .finish()
}

fn charge_canonical_string_bytes(scope: &ParseScope<'_>, bytes: usize) -> ParseResult<()> {
    scope
        .begin_work(
            ImportWorkClass::CanonicalStringBytes,
            usize_amount(scope, bytes, "obj-canonical-string-bytes")?,
        )?
        .complete()
}

fn charge_material_strings(scope: &ParseScope<'_>, parsed: &ParsedMaterial<'_>) -> ParseResult<()> {
    let mut bytes = parsed.name.len();
    let mut metadata = parsed.metadata.iter();
    mtl::for_work_chunks(
        scope,
        parsed.metadata.len(),
        "obj-material-string-scan-work",
        |range| {
            for _ in range {
                let (key, value) = metadata.next().ok_or_else(|| {
                    scope.format_failure(ObjFailure::internal("material-metadata-scan-ended-early"))
                })?;
                checked_add_string_bytes(scope, &mut bytes, key.len())?;
                if let ParsedMetadataValue::String(value) = value {
                    checked_add_string_bytes(scope, &mut bytes, value.len())?;
                }
            }
            Ok(())
        },
    )?;
    if let Some(texture) = &parsed.diffuse_texture {
        let scan_permit = scope.begin_work(ImportWorkClass::WorkUnits, 1)?;
        checked_add_string_bytes(scope, &mut bytes, texture.uri.len())?;
        checked_add_string_bytes(scope, &mut bytes, texture.uri.len())?;
        checked_add_string_bytes(scope, &mut bytes, texture.source_key.len())?;
        scan_permit.complete()?;
    }
    charge_canonical_string_bytes(scope, bytes)
}

fn clone_metadata<'scope>(
    scope: &'scope ParseScope<'_>,
    source: &ParsedMetadata<'_>,
) -> ParseResult<SceneValueParts<'scope, MetadataMap>> {
    let mut output = scope.metered_metadata_map("obj-material-metadata-map");
    for (key, value) in source.iter() {
        let key = canonical_string_parts(scope, key)?;
        let entry = match value {
            ParsedMetadataValue::I64(value) => key.into_metadata_i64_entry(*value),
            ParsedMetadataValue::F64(value) => key.into_metadata_f64_entry(*value),
            ParsedMetadataValue::String(value) => key
                .zip(canonical_string_parts(scope, value.as_str())?)?
                .into_metadata_string_entry(),
        };
        output.try_insert_parts(entry)?;
    }
    output.try_into_scene_parts()
}

fn compute_bounds(scope: &ParseScope<'_>, positions: &[Vec3]) -> ParseResult<Option<Aabb>> {
    let Some(first) = positions.first().copied() else {
        return Ok(None);
    };
    let mut min = first;
    let mut max = first;
    mtl::for_work_chunks(scope, positions.len(), "obj-bounds-work", |range| {
        for position in positions[range].iter().copied() {
            min.x = min.x.min(position.x);
            min.y = min.y.min(position.y);
            min.z = min.z.min(position.z);
            max.x = max.x.max(position.x);
            max.y = max.y.max(position.y);
            max.z = max.z.max(position.z);
        }
        Ok(())
    })?;
    Ok(Some(Aabb { min, max }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_with_object(object: StateNameId) -> SegmentKey {
        SegmentKey {
            object: Some(object),
            group: None,
            material: None,
            smoothing: None,
        }
    }

    #[test]
    fn segment_keys_compare_state_ids_in_constant_time() {
        assert_eq!(
            key_with_object(StateNameId::from_index(7)),
            key_with_object(StateNameId::from_index(7))
        );
        assert_ne!(
            key_with_object(StateNameId::from_index(7)),
            key_with_object(StateNameId::from_index(8))
        );
    }
}
