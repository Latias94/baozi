use crate::ObjFailure;
use crate::allocation::{LiveStorage, usize_to_u64 as usize_amount};
use crate::mtl::{self, MaterialLibrary, ParsedMaterial};
use crate::obj::{Face, FaceVertex, ParsedObj, StateNameId};
use baozi_core::{
    Aabb, AlphaMode, ColorSpace, Material, MaterialId, Mesh, MeshBinding, MetadataMap,
    MetadataValue, Node, PrimitiveTopology, SceneDraft, ShadingModel, Texture, TextureRole,
    TextureSlot, TextureSource, Vec2, Vec3,
};
use baozi_import::{
    ImportStorageClass, ImportWorkClass, ParseResult, ParseScope, ScopedSceneBuilder,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SegmentKey {
    object: Option<StateNameId>,
    group: Option<StateNameId>,
    material: Option<StateNameId>,
    smoothing: Option<StateNameId>,
}

impl SegmentKey {
    fn from_face(face: &Face) -> Self {
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
    vertex_keys: Vec<VertexKey>,
    face_vertex_counts: Vec<u32>,
    has_polygon: bool,
    storage: SegmentStorage<'scope>,
}

struct SegmentStorage<'scope> {
    vertex_keys: LiveStorage<'scope>,
    face_vertex_counts: LiveStorage<'scope>,
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
    storage: &mut LiveStorage<'scope>,
    value: &str,
) -> ParseResult<String> {
    storage.copy_string(scope, value, "obj-canonical-string")
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
    storage: &mut LiveStorage<'scope>,
    metadata: &mut MetadataMap,
    key: &'static str,
    value: &str,
) -> ParseResult<()> {
    let key = owned_canonical_string(scope, storage, key)?;
    let value = MetadataValue::String(owned_canonical_string(scope, storage, value)?);
    storage.insert_ordered_map(scope, metadata, key, value, "obj-mesh-metadata-map")?;
    Ok(())
}

impl<'scope> SegmentBuilder<'scope> {
    fn new(key: SegmentKey) -> Self {
        Self {
            key,
            vertex_keys: Vec::new(),
            face_vertex_counts: Vec::new(),
            has_polygon: false,
            storage: SegmentStorage {
                vertex_keys: LiveStorage::new(ImportStorageClass::FormatNormalization),
                face_vertex_counts: LiveStorage::new(ImportStorageClass::SceneConstruction),
            },
        }
    }

    fn push_face(&mut self, scope: &'scope ParseScope<'_>, face: &Face) -> ParseResult<()> {
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
        self.storage.face_vertex_counts.reserve_vec(
            scope,
            &mut self.face_vertex_counts,
            1,
            "obj-face-vertex-counts",
        )?;
        self.storage.vertex_keys.reserve_vec(
            scope,
            &mut self.vertex_keys,
            face_vertex_count,
            "obj-segment-vertex-keys",
        )?;
        self.face_vertex_counts.push(
            u32::try_from(face_vertex_count)
                .map_err(|_| scope.allocation_failure("obj-face-vertex-count"))?,
        );

        for chunk in face.vertices.chunks(64) {
            scope.poll()?;
            for vertex in chunk {
                self.vertex_keys.push((*vertex).into());
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

        let mut sorted_storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
        let mut sorted_keys = Vec::new();
        sorted_storage.reserve_vec(
            scope,
            &mut sorted_keys,
            self.vertex_keys.len(),
            "obj-sorted-vertex-keys",
        )?;
        for (chunk_index, chunk) in self.vertex_keys.chunks(64).enumerate() {
            scope.poll()?;
            let chunk_start = chunk_index
                .checked_mul(64)
                .ok_or_else(|| scope.allocation_failure("obj-vertex-key-copy-offset"))?;
            for (relative, key) in chunk.iter().copied().enumerate() {
                let occurrence = chunk_start
                    .checked_add(relative)
                    .ok_or_else(|| scope.allocation_failure("obj-vertex-key-copy-offset"))?;
                sorted_keys.push(IndexedVertexKey { key, occurrence });
            }
        }
        mtl::interruptible_sort_by(scope, &mut sorted_keys, |_work, left, right| {
            Ok(left.cmp(right))
        })?;
        let mut first_storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
        let mut first_occurrences = Vec::new();
        first_storage.reserve_vec(
            scope,
            &mut first_occurrences,
            self.vertex_keys.len(),
            "obj-first-vertex-occurrences",
        )?;
        mtl::for_work_chunks(
            scope,
            self.vertex_keys.len(),
            "obj-first-vertex-occurrence-init-work",
            |range| {
                first_occurrences.resize(range.end, false);
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
                    let indexed = sorted_keys[cursor];
                    let is_first = previous_key != Some(indexed.key);
                    previous_key = Some(indexed.key);
                    if !is_first {
                        continue;
                    }
                    first_occurrences[indexed.occurrence] = true;
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
        let mut positions_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
        let mut texcoords_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
        let mut normals_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
        let mut indices_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
        let mut positions = Vec::new();
        let mut texcoords = Vec::new();
        let mut normals = Vec::new();
        let mut indices = Vec::new();
        positions_storage.reserve_vec(
            scope,
            &mut positions,
            unique_count,
            "obj-segment-positions",
        )?;
        if keep_texcoords {
            texcoords_storage.reserve_vec(
                scope,
                &mut texcoords,
                unique_count,
                "obj-segment-texcoords",
            )?;
        }
        if keep_normals {
            normals_storage.reserve_vec(
                scope,
                &mut normals,
                unique_count,
                "obj-segment-normals",
            )?;
        }
        indices_storage.reserve_vec(
            scope,
            &mut indices,
            self.vertex_keys.len(),
            "obj-segment-indices",
        )?;
        mtl::for_work_chunks(
            scope,
            self.vertex_keys.len(),
            "obj-vertex-index-init-work",
            |range| {
                indices.resize(range.end, 0);
                Ok(())
            },
        )?;

        mtl::for_work_chunks(
            scope,
            self.vertex_keys.len(),
            "obj-vertex-materialization-work",
            |range| {
                for occurrence in range {
                    let key = &self.vertex_keys[occurrence];
                    if !first_occurrences[occurrence] {
                        continue;
                    }
                    let canonical_index = u32::try_from(positions.len())
                        .map_err(|_| scope.allocation_failure("obj-vertex-index-space"))?;
                    indices[occurrence] = canonical_index;
                    positions.push(parsed.positions[key.position]);
                    if keep_texcoords {
                        let Some(index) = key.texcoord else {
                            return Err(scope.format_failure(ObjFailure::internal(
                                "complete-texcoord-channel-has-missing-index",
                            )));
                        };
                        texcoords.push(parsed.texcoords[index]);
                    }
                    if keep_normals {
                        let Some(index) = key.normal else {
                            return Err(scope.format_failure(ObjFailure::internal(
                                "complete-normal-channel-has-missing-index",
                            )));
                        };
                        normals.push(parsed.normals[index]);
                    }
                }
                Ok(())
            },
        )?;
        let mut current_key = None;
        let mut canonical_index = 0u32;
        mtl::for_work_chunks(scope, sorted_keys.len(), "obj-vertex-remap-work", |range| {
            for cursor in range {
                let indexed = sorted_keys[cursor];
                if current_key != Some(indexed.key) {
                    current_key = Some(indexed.key);
                    canonical_index = indices[indexed.occurrence];
                } else {
                    indices[indexed.occurrence] = canonical_index;
                }
            }
            Ok(())
        })?;

        attribute_permit.complete()?;
        vertex_permit.complete()?;
        drop(first_occurrences);
        drop(first_storage);
        drop(sorted_keys);
        drop(sorted_storage);
        Ok(BuiltVertexStreams {
            positions,
            texcoords,
            normals,
            indices,
            storage: VertexStreamStorage {
                positions: positions_storage,
                texcoords: keep_texcoords.then_some(texcoords_storage),
                normals: keep_normals.then_some(normals_storage),
                indices: indices_storage,
            },
        })
    }

    fn finish(
        self,
        scope: &'scope ParseScope<'_>,
        parsed: &ParsedObj<'_>,
        material: Option<MaterialId>,
    ) -> ParseResult<FinishedSegment<'scope>> {
        let streams = self.materialize_vertices(scope, parsed)?;
        let keep_face_counts = self.has_polygon;
        let SegmentBuilder {
            key,
            vertex_keys,
            face_vertex_counts,
            has_polygon,
            storage,
        } = self;
        let SegmentStorage {
            vertex_keys: vertex_keys_storage,
            face_vertex_counts: face_counts_storage,
        } = storage;
        drop(vertex_keys);
        drop(vertex_keys_storage);
        let BuiltVertexStreams {
            positions,
            texcoords,
            normals,
            indices,
            storage: stream_storage,
        } = streams;
        let mut dynamic_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
        let name = if let Some(id) = key.display_name() {
            Some(owned_canonical_string(
                scope,
                &mut dynamic_storage,
                state_name(scope, parsed, id)?,
            )?)
        } else {
            None
        };
        let topology = if has_polygon {
            PrimitiveTopology::Polygons
        } else {
            PrimitiveTopology::Triangles
        };
        let (face_vertex_counts, face_counts_storage) = if keep_face_counts {
            (face_vertex_counts, Some(face_counts_storage))
        } else {
            drop(face_vertex_counts);
            drop(face_counts_storage);
            (Vec::new(), None)
        };
        let mut metadata = MetadataMap::new();
        if let Some(id) = key.object {
            insert_string_metadata(
                scope,
                &mut dynamic_storage,
                &mut metadata,
                "obj:object",
                state_name(scope, parsed, id)?,
            )?;
        }
        if let Some(id) = key.group {
            insert_string_metadata(
                scope,
                &mut dynamic_storage,
                &mut metadata,
                "obj:group",
                state_name(scope, parsed, id)?,
            )?;
        }
        if let Some(id) = key.material {
            insert_string_metadata(
                scope,
                &mut dynamic_storage,
                &mut metadata,
                "obj:material",
                state_name(scope, parsed, id)?,
            )?;
        }
        if let Some(id) = key.smoothing {
            insert_string_metadata(
                scope,
                &mut dynamic_storage,
                &mut metadata,
                "obj:smoothing",
                state_name(scope, parsed, id)?,
            )?;
        }
        let texcoords = if texcoords.is_empty() {
            Vec::new()
        } else {
            let mut channels = Vec::new();
            dynamic_storage.reserve_vec(
                scope,
                &mut channels,
                1,
                "obj-texture-coordinate-channels",
            )?;
            channels.push(texcoords);
            channels
        };
        let bounds = compute_bounds(scope, &positions)?;

        Ok(FinishedSegment {
            mesh: Mesh {
                name,
                topology,
                bounds,
                positions,
                normals,
                texcoords,
                indices,
                face_vertex_counts,
                material,
                metadata,
                ..Mesh::default()
            },
            storage: FinishedSegmentStorage {
                streams: stream_storage,
                face_vertex_counts: face_counts_storage,
                dynamic: dynamic_storage,
            },
        })
    }
}

struct BuiltVertexStreams<'scope> {
    positions: Vec<Vec3>,
    texcoords: Vec<Vec2>,
    normals: Vec<Vec3>,
    indices: Vec<u32>,
    storage: VertexStreamStorage<'scope>,
}

struct VertexStreamStorage<'scope> {
    positions: LiveStorage<'scope>,
    texcoords: Option<LiveStorage<'scope>>,
    normals: Option<LiveStorage<'scope>>,
    indices: LiveStorage<'scope>,
}

impl VertexStreamStorage<'_> {
    fn adopt(self, builder: &mut ScopedSceneBuilder<'_, '_>) -> ParseResult<()> {
        self.positions.adopt(builder)?;
        if let Some(storage) = self.texcoords {
            storage.adopt(builder)?;
        }
        if let Some(storage) = self.normals {
            storage.adopt(builder)?;
        }
        self.indices.adopt(builder)
    }
}

struct FinishedSegment<'scope> {
    mesh: Mesh,
    storage: FinishedSegmentStorage<'scope>,
}

struct FinishedSegmentStorage<'scope> {
    streams: VertexStreamStorage<'scope>,
    face_vertex_counts: Option<LiveStorage<'scope>>,
    dynamic: LiveStorage<'scope>,
}

impl FinishedSegmentStorage<'_> {
    fn adopt(self, builder: &mut ScopedSceneBuilder<'_, '_>) -> ParseResult<()> {
        self.streams.adopt(builder)?;
        if let Some(storage) = self.face_vertex_counts {
            storage.adopt(builder)?;
        }
        self.dynamic.adopt(builder)
    }
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

    for face in &parsed.faces {
        let key = SegmentKey::from_face(face);
        let should_flush = current.as_ref().is_some_and(|segment| segment.key != key);
        if should_flush && let Some(segment) = current.take() {
            flush_segment(scope, &mut builder, &material_bindings, &parsed, segment)?;
        }
        if current.is_none() {
            current = Some(SegmentBuilder::new(key));
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
    let finished = segment.finish(scope, parsed, material)?;
    let mut node_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
    let name = finished
        .mesh
        .name
        .as_deref()
        .map(|value| owned_canonical_string(scope, &mut node_storage, value))
        .transpose()?;
    finished.storage.adopt(builder)?;
    let mesh_id = builder.add_mesh(finished.mesh)?;
    let mut mesh_bindings = Vec::new();
    node_storage.reserve_vec(scope, &mut mesh_bindings, 1, "obj-node-mesh-bindings")?;
    mesh_bindings.push(MeshBinding::new(mesh_id));
    node_storage.adopt(builder)?;
    builder.add_child_node(
        builder.root(),
        Node {
            name,
            mesh_bindings,
            ..Node::default()
        },
    )?;
    mesh_permit.complete()?;
    Ok(())
}

struct MaterialBinding<'parsed> {
    name: &'parsed str,
    first_use: usize,
    id: Option<MaterialId>,
}

struct MaterialBindings<'scope, 'parsed> {
    values: Vec<MaterialBinding<'parsed>>,
    _storage: LiveStorage<'scope>,
}

impl MaterialBindings<'_, '_> {
    fn find(&self, scope: &ParseScope<'_>, name: &str) -> ParseResult<Option<MaterialId>> {
        let index = mtl::interruptible_binary_search_by(scope, &self.values, |work, binding| {
            work.compare_str(binding.name, name)
        })?;
        Ok(index.and_then(|index| self.values[index].id))
    }
}

fn prepare_material_bindings<'scope, 'parsed>(
    scope: &'scope ParseScope<'_>,
    builder: &mut ScopedSceneBuilder<'_, '_>,
    library: &MaterialLibrary<'scope>,
    parsed: &'parsed ParsedObj<'_>,
) -> ParseResult<MaterialBindings<'scope, 'parsed>> {
    let faces = &parsed.faces;
    let mut storage = LiveStorage::new(ImportStorageClass::FormatNormalization);
    let mut values = Vec::new();
    mtl::for_work_chunks(
        scope,
        faces.len(),
        "obj-material-binding-scan-work",
        |range| {
            for first_use in range {
                let Some(id) = faces[first_use].material else {
                    continue;
                };
                storage.reserve_vec(scope, &mut values, 1, "obj-material-bindings")?;
                values.push(MaterialBinding {
                    name: state_name(scope, parsed, id)?,
                    first_use,
                    id: None,
                });
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
                let name = values[index].name;
                let id = build_material(scope, builder, library, name)?;
                values[index].id = Some(id);
            }
            Ok(())
        },
    )?;
    mtl::interruptible_sort_by(scope, &mut values, |work, left, right| {
        work.compare_str(left.name, right.name)
    })?;
    Ok(MaterialBindings {
        values,
        _storage: storage,
    })
}

fn build_material<'scope>(
    scope: &'scope ParseScope<'_>,
    builder: &mut ScopedSceneBuilder<'_, '_>,
    library: &MaterialLibrary<'scope>,
    name: &str,
) -> ParseResult<MaterialId> {
    let built = if let Some(parsed) = library.find(scope, name)? {
        material_from_parsed(scope, builder, parsed)?
    } else {
        mtl::push_warning(
            scope,
            None,
            "obj.material_missing",
            format_args!("OBJ material '{name}' was referenced but not defined"),
        )?;
        charge_canonical_string_bytes(scope, name.len())?;
        let mut storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
        BuiltMaterial {
            material: Material {
                name: Some(owned_canonical_string(scope, &mut storage, name)?),
                ..Material::default()
            },
            storage,
        }
    };
    built.storage.adopt(builder)?;
    builder.add_material(built.material)
}

struct BuiltMaterial<'scope> {
    material: Material,
    storage: LiveStorage<'scope>,
}

fn material_from_parsed<'scope>(
    scope: &'scope ParseScope<'_>,
    builder: &mut ScopedSceneBuilder<'_, '_>,
    parsed: &ParsedMaterial<'_>,
) -> ParseResult<BuiltMaterial<'scope>> {
    charge_material_strings(scope, parsed)?;
    let mut storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
    let mut material = Material {
        name: Some(owned_canonical_string(scope, &mut storage, &parsed.name)?),
        shading_model: ShadingModel::Phong,
        base_color: parsed.base_color,
        emissive: parsed.emissive,
        alpha_mode: parsed.alpha_mode,
        metadata: clone_metadata(scope, &mut storage, &parsed.metadata)?,
        ..Material::default()
    };

    if parsed.base_color.a < 1.0 {
        material.alpha_mode = AlphaMode::Blend;
    }

    if let Some(texture) = &parsed.diffuse_texture {
        let mut texture_storage = LiveStorage::new(ImportStorageClass::SceneConstruction);
        let texture_value = Texture {
            name: Some(owned_canonical_string(
                scope,
                &mut texture_storage,
                &texture.uri,
            )?),
            source: TextureSource::External {
                uri: owned_canonical_string(scope, &mut texture_storage, &texture.uri)?,
            },
            sampler: Default::default(),
            metadata: Default::default(),
        };
        texture_storage.adopt(builder)?;
        let texture_id = builder.add_texture(texture_value)?;
        storage.reserve_vec(
            scope,
            &mut material.texture_slots,
            1,
            "obj-material-texture-slots",
        )?;
        material.texture_slots.push(TextureSlot {
            texture: texture_id,
            role: TextureRole::Diffuse,
            color_space: ColorSpace::Srgb,
            uv_set: 0,
            scale: 1.0,
            transform: Default::default(),
            source_key: Some(owned_canonical_string(
                scope,
                &mut storage,
                texture.source_key,
            )?),
        });
    }

    Ok(BuiltMaterial { material, storage })
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
                if let MetadataValue::String(value) = value {
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
    storage: &mut LiveStorage<'scope>,
    source: &MetadataMap,
) -> ParseResult<MetadataMap> {
    let mut output = MetadataMap::new();
    for (key, value) in source {
        let key = owned_canonical_string(scope, storage, key)?;
        let value = match value {
            MetadataValue::Bool(value) => MetadataValue::Bool(*value),
            MetadataValue::I64(value) => MetadataValue::I64(*value),
            MetadataValue::F64(value) => MetadataValue::F64(*value),
            MetadataValue::String(value) => {
                MetadataValue::String(owned_canonical_string(scope, storage, value)?)
            }
        };
        storage.insert_ordered_map(scope, &mut output, key, value, "obj-material-metadata-map")?;
    }
    Ok(output)
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
