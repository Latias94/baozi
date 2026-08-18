use crate::mtl::{
    self, MaterialLibrary, ParsedAlphaMode, ParsedMaterial, ParsedMetadata, ParsedMetadataValue,
    ParsedTexture,
};
use crate::obj::{Face, FaceVertex, ParsedObj, StateNameId};
use crate::parser::usize_amount;
use crate::{FORMAT_ID, ObjFailure, diffuse_texture_role};
use baozi_import::{
    AlphaMode as CanonicalAlphaMode, AnimationAuthoringBatch, AppearanceAuthoringBatch,
    AttributeDomain, AttributeSemantic, AuthoredTransform, AuthoringRange, Color,
    ColorInterpretation, DeformationAuthoringBatch, DependencySnapshot, ExtensionValueRef,
    FormatId, GeometryAttributeAuthoring, GeometryAuthoringBatch, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    ImageAuthoring, ImageLocalId, ImportStorageClass, ImportWorkClass, LegacyMaterialFactors,
    LineageAuthoringBatch, LogicalMeshLocalId, LogicalMorphAuthoring, MAX_ENTITY_NAME_BYTES,
    MaterialAuthoring, MaterialBindingAuthoring, MaterialFactors, MaterialLocalId,
    MeshInstanceDeformationAuthoring, MetadataAuthoringBatch, MetadataAuthoringEntry, MeteredVec,
    NamespacedName, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, ObservedResourceAuthoring,
    OwnerProvenanceAuthoring, ParseResult, ParseScope, Presence, PrimitiveDeformationAuthoring,
    ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring,
    ResourceIdentity, ResourceIdentityLocalId, ResourceSnapshotAuthoring, SamplerAuthoring,
    SamplerLocalId, SamplerState, SceneAuthoringBatch, SceneDraft, SceneSpace, SessionOpaqueReason,
    ShadingModel as CanonicalShadingModel, SourceIdentityAuthoring, SourceIdentityLocalId,
    SourceObjectKind, SourceObjectLocator, SpatialAuthoringBatch, TextureAuthoring,
    TextureBlendOperation, TextureChannelMapping, TextureChannelSwizzle, TextureCoordinateSource,
    TextureLocalId, TextureRole, TextureSourceKey, TextureStrength, TextureTransform, Vec3,
};

const FALLBACK_RESOURCE_IDENTITY: &str = "session/obj-primary";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SegmentKey {
    object: Option<StateNameId>,
    group: Option<StateNameId>,
    material: Option<StateNameId>,
    smoothing: Option<StateNameId>,
}

fn bounded_entity_name(name: &str) -> Option<&str> {
    (name.len() <= MAX_ENTITY_NAME_BYTES).then_some(name)
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

impl<'scope> SegmentBuilder<'scope> {
    fn new(scope: &'scope ParseScope<'_>, key: SegmentKey) -> Self {
        Self {
            key,
            vertex_keys: scope.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-segment-vertex-keys",
            ),
            face_vertex_counts: scope.metered_vec(
                ImportStorageClass::FormatNormalization,
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
        let canonical_topology_cells = usize_amount(
            scope,
            canonical_topology_cells,
            "obj-canonical-topology-cells",
        )?;
        let face_count_cells =
            usize_amount(scope, face_count_cells, "obj-canonical-face-count-cells")?;

        scope.with_work(
            ImportWorkClass::TopologyCells,
            canonical_topology_cells,
            |work| -> ParseResult<()> {
                if face_vertex_count > 3 {
                    self.has_polygon = true;
                }
                let face_vertex_count = u32::try_from(face_vertex_count)
                    .map_err(|_| scope.allocation_failure("obj-face-vertex-count"))?;
                work.run([face_count_cells], || {
                    self.face_vertex_counts.try_push(face_vertex_count)
                })?;
                self.vertex_keys.try_reserve(face.vertices.len())?;

                for chunk in face.vertices.as_slice().chunks(64) {
                    work.poll()?;
                    for vertex in chunk {
                        work.run([1], || self.vertex_keys.try_push_reserved((*vertex).into()))?;
                    }
                }
                Ok(())
            },
        )
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
                sorted_keys.try_push_reserved(IndexedVertexKey { key, occurrence })?;
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

        let channel_count = usize::from(keep_texcoords) + usize::from(keep_normals);
        let channel_count_amount =
            usize_amount(scope, channel_count, "obj-canonical-attribute-stream-count")?;
        let mut positions = scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-segment-position-scalars",
        );
        let mut texcoords = scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-segment-texcoord-scalars",
        );
        let mut normals = scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-segment-normal-scalars",
        );
        let mut indices = scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-segment-indices",
        );
        positions.try_reserve(
            unique_count
                .checked_mul(3)
                .ok_or_else(|| scope.allocation_failure("obj-position-scalar-count"))?,
        )?;
        if keep_texcoords {
            texcoords.try_reserve(
                unique_count
                    .checked_mul(2)
                    .ok_or_else(|| scope.allocation_failure("obj-texcoord-scalar-count"))?,
            )?;
        }
        if keep_normals {
            normals.try_reserve(
                unique_count
                    .checked_mul(3)
                    .ok_or_else(|| scope.allocation_failure("obj-normal-scalar-count"))?,
            )?;
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

        for (chunk_index, first_flags) in first_occurrences.as_slice().chunks(64).enumerate() {
            let start = chunk_index
                .checked_mul(64)
                .ok_or_else(|| scope.allocation_failure("obj-vertex-materialization-offset"))?;
            let end = start
                .checked_add(first_flags.len())
                .ok_or_else(|| scope.allocation_failure("obj-vertex-materialization-offset"))?;
            let chunk_vertices = first_flags.iter().filter(|is_first| **is_first).count();
            let chunk_attribute_cells = chunk_vertices
                .checked_mul(channel_count)
                .ok_or_else(|| scope.allocation_failure("obj-canonical-attribute-cells"))?;
            scope.with_work_group(
                [
                    (
                        ImportWorkClass::WorkUnits,
                        usize_amount(scope, first_flags.len(), "obj-vertex-materialization-work")?,
                    ),
                    (
                        ImportWorkClass::Vertices,
                        usize_amount(scope, chunk_vertices, "obj-canonical-vertices")?,
                    ),
                    (
                        ImportWorkClass::AttributeCells,
                        usize_amount(
                            scope,
                            chunk_attribute_cells,
                            "obj-canonical-attribute-cells",
                        )?,
                    ),
                ],
                |work| -> ParseResult<()> {
                    for occurrence in start..end {
                        let key = &self.vertex_keys.as_slice()[occurrence];
                        if !first_occurrences.as_slice()[occurrence] {
                            work.complete([1, 0, 0])?;
                            continue;
                        }
                        let canonical_index = u32::try_from(positions.len() / 3)
                            .map_err(|_| scope.allocation_failure("obj-vertex-index-space"))?;
                        let position = parsed.positions.as_slice()[key.position];
                        let texcoord = if keep_texcoords {
                            let Some(index) = key.texcoord else {
                                return Err(scope.format_failure(ObjFailure::internal(
                                    "complete-texcoord-channel-has-missing-index",
                                )));
                            };
                            Some(parsed.texcoords.as_slice()[index])
                        } else {
                            None
                        };
                        let normal = if keep_normals {
                            let Some(index) = key.normal else {
                                return Err(scope.format_failure(ObjFailure::internal(
                                    "complete-normal-channel-has-missing-index",
                                )));
                            };
                            Some(parsed.normals.as_slice()[index])
                        } else {
                            None
                        };
                        work.run([1, 1, channel_count_amount], || {
                            indices.as_mut_slice()[occurrence] = canonical_index;
                            positions.try_extend_from_slice_reserved(&[
                                position.x, position.y, position.z,
                            ])?;
                            if let Some(texcoord) = texcoord {
                                texcoords
                                    .try_extend_from_slice_reserved(&[texcoord.x, texcoord.y])?;
                            }
                            if let Some(normal) = normal {
                                normals.try_extend_from_slice_reserved(&[
                                    normal.x, normal.y, normal.z,
                                ])?;
                            }
                            Ok(())
                        })?;
                    }
                    Ok(())
                },
            )?;
        }
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
        material: Option<MaterialLocalId>,
    ) -> ParseResult<BuiltSegment<'scope>> {
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
        let face_vertex_counts = if has_polygon {
            face_vertex_counts
        } else {
            drop(face_vertex_counts);
            scope.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-face-vertex-counts",
            )
        };
        Ok(BuiltSegment {
            key,
            positions,
            texcoords,
            normals,
            indices,
            face_vertex_counts,
            has_polygon,
            material,
        })
    }
}

struct BuiltVertexStreams<'scope> {
    positions: MeteredVec<'scope, f32>,
    texcoords: MeteredVec<'scope, f32>,
    normals: MeteredVec<'scope, f32>,
    indices: MeteredVec<'scope, u32>,
}

struct BuiltSegment<'scope> {
    key: SegmentKey,
    positions: MeteredVec<'scope, f32>,
    texcoords: MeteredVec<'scope, f32>,
    normals: MeteredVec<'scope, f32>,
    indices: MeteredVec<'scope, u32>,
    face_vertex_counts: MeteredVec<'scope, u32>,
    has_polygon: bool,
    material: Option<MaterialLocalId>,
}

pub(crate) fn scene_from_parsed<'scope>(
    scope: &'scope ParseScope<'_>,
    parsed: ParsedObj<'scope>,
    library: &MaterialLibrary<'scope>,
) -> ParseResult<SceneDraft> {
    if parsed.faces.is_empty() {
        return Err(scope.format_failure(ObjFailure::empty_geometry()));
    }

    let material_projection = prepare_material_bindings(scope, library, &parsed)?;
    // The complete batch is borrowed authoring scratch. Core alone owns canonical-output
    // reservations when activation materializes and jointly publishes the retained Scene.
    let mut segments = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-segments",
    );
    let mut current: Option<SegmentBuilder<'scope>> = None;

    for face in parsed.faces.as_slice() {
        let key = SegmentKey::from_face(face);
        let should_flush = current.as_ref().is_some_and(|segment| segment.key != key);
        if should_flush && let Some(segment) = current.take() {
            flush_segment(
                scope,
                &mut segments,
                &material_projection.bindings,
                &parsed,
                segment,
            )?;
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
        flush_segment(
            scope,
            &mut segments,
            &material_projection.bindings,
            &parsed,
            segment,
        )?;
    }

    submit_complete_batch(scope, &parsed, material_projection.materials, segments)
}

fn flush_segment<'scope>(
    scope: &'scope ParseScope<'_>,
    segments: &mut MeteredVec<'scope, BuiltSegment<'scope>>,
    material_bindings: &MaterialBindings<'scope, '_>,
    parsed: &ParsedObj<'_>,
    segment: SegmentBuilder<'scope>,
) -> ParseResult<()> {
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
    let built = segment.finish(scope, parsed, material)?;
    scope.with_work(ImportWorkClass::Meshes, 1, |work| {
        work.run_all(|| segments.try_push(built))
    })
}

#[derive(Clone, Copy)]
struct MaterialBinding<'parsed> {
    name: &'parsed str,
    first_use: usize,
    id: Option<MaterialLocalId>,
}

struct MaterialBindings<'scope, 'parsed> {
    values: MeteredVec<'scope, MaterialBinding<'parsed>>,
}

impl MaterialBindings<'_, '_> {
    fn find(&self, scope: &ParseScope<'_>, name: &str) -> ParseResult<Option<MaterialLocalId>> {
        let index =
            mtl::interruptible_binary_search_by(scope, self.values.as_slice(), |work, binding| {
                work.compare_str(binding.name, name)
            })?;
        Ok(index.and_then(|index| self.values.as_slice()[index].id))
    }
}

struct BuiltMaterial<'input, 'scope> {
    name: &'input str,
    shading_model: CanonicalShadingModel,
    alpha_mode: CanonicalAlphaMode,
    factors: MaterialFactors,
    metadata: Option<&'input ParsedMetadata<'scope>>,
    diffuse_texture: Option<BuiltTexture<'scope>>,
}

struct BuiltTexture<'scope> {
    identity: ResourceIdentity,
    snapshot: DependencySnapshot<'scope>,
}

struct MaterialProjection<'scope, 'input> {
    bindings: MaterialBindings<'scope, 'input>,
    materials: MeteredVec<'scope, BuiltMaterial<'input, 'scope>>,
}

fn prepare_material_bindings<'scope, 'input>(
    scope: &'scope ParseScope<'_>,
    library: &'input MaterialLibrary<'scope>,
    parsed: &'input ParsedObj<'scope>,
) -> ParseResult<MaterialProjection<'scope, 'input>> {
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

    let mut materials = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-materials",
    );
    materials.try_reserve(values.len())?;
    let mut start = 0usize;
    while start < values.len() {
        let end = start.saturating_add(64).min(values.len());
        let mut built_ids: [Option<MaterialLocalId>; 64] = std::array::from_fn(|_| None);
        for (offset, index) in (start..end).enumerate() {
            let name = values.as_slice()[index].name;
            let material = build_material(scope, library, name)?;
            let id = canonical_u32(scope, materials.len(), "obj-material-local-id")?;
            materials.try_push_reserved(material)?;
            built_ids[offset] = Some(MaterialLocalId::new(id));
        }
        scope.with_work(
            ImportWorkClass::WorkUnits,
            usize_amount(scope, end - start, "obj-material-binding-build-work")?,
            |work| -> ParseResult<()> {
                for (offset, index) in (start..end).enumerate() {
                    let id = built_ids[offset].take().ok_or_else(|| {
                        scope.format_failure(ObjFailure::internal(
                            "material-binding-build-result-missing",
                        ))
                    })?;
                    work.run([1], || {
                        values.as_mut_slice()[index].id = Some(id);
                        Ok(())
                    })?;
                }
                Ok(())
            },
        )?;
        start = end;
    }
    mtl::interruptible_sort_by(scope, &mut values, |work, left, right| {
        work.compare_str(left.name, right.name)
    })?;
    Ok(MaterialProjection {
        bindings: MaterialBindings { values },
        materials,
    })
}

fn build_material<'input, 'scope>(
    scope: &'scope ParseScope<'_>,
    library: &'input MaterialLibrary<'scope>,
    name: &'input str,
) -> ParseResult<BuiltMaterial<'input, 'scope>> {
    if let Some(parsed) = library.find(scope, name)? {
        material_from_parsed(scope, library, parsed)
    } else {
        mtl::push_warning(
            scope,
            None,
            "obj.material_missing",
            format_args!("OBJ material '{name}' was referenced but not defined"),
        )?;
        material_from_factors(
            scope,
            name,
            CanonicalShadingModel::Phong,
            CanonicalAlphaMode::Opaque,
            Color::WHITE,
            Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
            0.0,
            None,
            None,
        )
    }
}

fn material_from_parsed<'input, 'scope>(
    scope: &'scope ParseScope<'_>,
    library: &'input MaterialLibrary<'scope>,
    parsed: &'input ParsedMaterial<'scope>,
) -> ParseResult<BuiltMaterial<'input, 'scope>> {
    let diffuse_texture = parsed
        .diffuse_texture
        .as_ref()
        .map(|texture| load_diffuse_texture(scope, library, texture))
        .transpose()?
        .flatten();

    let alpha_mode = if parsed.base_color.a < 1.0 {
        CanonicalAlphaMode::Blend
    } else {
        match parsed.alpha_mode {
            ParsedAlphaMode::Opaque => CanonicalAlphaMode::Opaque,
            ParsedAlphaMode::Blend => CanonicalAlphaMode::Blend,
        }
    };

    material_from_factors(
        scope,
        parsed.name.as_str(),
        CanonicalShadingModel::Phong,
        alpha_mode,
        parsed.base_color,
        parsed.emissive,
        parsed_shininess(&parsed.metadata),
        Some(&parsed.metadata),
        diffuse_texture,
    )
}

fn load_diffuse_texture<'scope>(
    scope: &'scope ParseScope<'_>,
    library: &MaterialLibrary<'scope>,
    texture: &ParsedTexture<'_>,
) -> ParseResult<Option<BuiltTexture<'scope>>> {
    let origin = library
        .origin(texture.origin)
        .ok_or_else(|| scope.format_failure(ObjFailure::internal("texture-origin-missing")))?;
    let outcome = scope.dependency(origin, &diffuse_texture_role(), texture.reference.as_str())?;
    let snapshot = match outcome.into_result() {
        Ok(snapshot) => snapshot,
        Err(evidence) => {
            mtl::push_warning(
                scope,
                None,
                "obj.mtl_texture_unavailable",
                format_args!(
                    "optional diffuse texture '{}' is unavailable; owner={:?}, cache={:?}",
                    evidence.normalized_path(),
                    evidence.owner(),
                    evidence.cache_disposition()
                ),
            )?;
            return Ok(None);
        }
    };
    let identity = ResourceIdentity::new(snapshot.identity().path().as_str())
        .map_err(|_| scope.format_failure(ObjFailure::internal("texture-identity-invalid")))?;
    Ok(Some(BuiltTexture { identity, snapshot }))
}

#[allow(clippy::too_many_arguments)]
fn material_from_factors<'input, 'scope>(
    scope: &ParseScope<'_>,
    name: &'input str,
    shading_model: CanonicalShadingModel,
    alpha_mode: CanonicalAlphaMode,
    base_color: Color,
    emissive: Color,
    shininess: f32,
    metadata: Option<&'input ParsedMetadata<'scope>>,
    diffuse_texture: Option<BuiltTexture<'scope>>,
) -> ParseResult<BuiltMaterial<'input, 'scope>> {
    let legacy = LegacyMaterialFactors::try_new(
        Vec3::ZERO,
        Vec3::new(base_color.r, base_color.g, base_color.b),
        Vec3::ZERO,
        Vec3::new(emissive.r, emissive.g, emissive.b),
        shininess,
        base_color.a,
    )
    .map_err(|_| scope.format_failure(ObjFailure::internal("invalid-material-factors")))?;
    let factors = MaterialFactors::try_new(
        Vec3::new(emissive.r, emissive.g, emissive.b),
        None,
        None,
        Some(legacy),
    )
    .map_err(|_| scope.format_failure(ObjFailure::internal("invalid-material-factors")))?;
    Ok(BuiltMaterial {
        name,
        shading_model,
        alpha_mode,
        factors,
        metadata,
        diffuse_texture,
    })
}

fn parsed_shininess(metadata: &ParsedMetadata<'_>) -> f32 {
    metadata
        .iter()
        .find_map(|(key, value)| match (key, value) {
            ("obj:ns", ParsedMetadataValue::F64(value)) => Some(*value as f32),
            _ => None,
        })
        .unwrap_or(0.0)
}

#[derive(Clone, Copy)]
enum BorrowedMetadataValue<'input> {
    I64(i64),
    F64(f64),
    String(&'input str),
}

impl<'input> BorrowedMetadataValue<'input> {
    const fn as_extension(self) -> ExtensionValueRef<'input> {
        match self {
            Self::I64(value) => ExtensionValueRef::I64(value),
            Self::F64(value) => ExtensionValueRef::F64(value),
            Self::String(value) => ExtensionValueRef::String(value),
        }
    }
}

struct PendingMetadata<'input> {
    owner: ProfileOwner,
    key: NamespacedName,
    value: BorrowedMetadataValue<'input>,
}

fn submit_complete_batch<'scope>(
    scope: &'scope ParseScope<'_>,
    parsed: &ParsedObj<'scope>,
    materials: MeteredVec<'scope, BuiltMaterial<'_, 'scope>>,
    segments: MeteredVec<'scope, BuiltSegment<'scope>>,
) -> ParseResult<SceneDraft> {
    let segment_count = segments.len();
    let mut roots = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-roots",
    );
    let mut nodes = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-nodes",
    );
    let mut logical_meshes = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-logical-meshes",
    );
    let mut primitives = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-primitives",
    );
    let mut attributes = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-attributes",
    );
    let mut mesh_instances = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-mesh-instances",
    );
    let mut logical_morphs = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-logical-morphs",
    );
    let mut primitive_deformations = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-primitive-deformations",
    );
    let mut mesh_instance_deformations = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-instance-deformations",
    );

    roots.try_reserve(segment_count)?;
    nodes.try_reserve(segment_count)?;
    logical_meshes.try_reserve(segment_count)?;
    primitives.try_reserve(segment_count)?;
    mesh_instances.try_reserve(segment_count)?;
    logical_morphs.try_reserve(segment_count)?;
    primitive_deformations.try_reserve(segment_count)?;
    mesh_instance_deformations.try_reserve(segment_count)?;
    let attribute_count = segments
        .as_slice()
        .iter()
        .try_fold(0usize, |total, segment| {
            total
                .checked_add(
                    1 + usize::from(!segment.normals.is_empty())
                        + usize::from(!segment.texcoords.is_empty()),
                )
                .ok_or_else(|| scope.allocation_failure("obj-attribute-count"))
        })?;
    attributes.try_reserve(attribute_count)?;

    for (index, segment) in segments.as_slice().iter().enumerate() {
        let local = canonical_u32(scope, index, "obj-node-local-id")?;
        let node_id = NodeLocalId::new(local);
        roots.try_push_reserved(node_id)?;
        let mut node = NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        );
        if let Some(name) = segment
            .key
            .display_name()
            .map(|id| state_name(scope, parsed, id))
            .transpose()?
            .and_then(bounded_entity_name)
        {
            node = node.with_name(name);
        }
        nodes.try_push_reserved(node)?;

        let attribute_start = attributes.len();
        let attribute_offset = canonical_u32(scope, attribute_start, "obj-attribute-offset")?;
        attributes.try_push_reserved(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            segment.positions.as_slice(),
            Presence::Absent,
        ))?;
        if !segment.normals.is_empty() {
            attributes.try_push_reserved(GeometryAttributeAuthoring::f32(
                AttributeSemantic::Normal,
                AttributeDomain::Vertex,
                3,
                segment.normals.as_slice(),
                Presence::Absent,
            ))?;
        }
        if !segment.texcoords.is_empty() {
            attributes.try_push_reserved(GeometryAttributeAuthoring::f32(
                AttributeSemantic::Texcoord(0),
                AttributeDomain::Vertex,
                2,
                segment.texcoords.as_slice(),
                Presence::Absent,
            ))?;
        }
        let attribute_range = AuthoringRange::new(
            attribute_offset,
            canonical_u32(
                scope,
                attributes.len() - attribute_start,
                "obj-attribute-range-count",
            )?,
        );
        let topology = if segment.has_polygon {
            GeometryTopologyAuthoring::Polygons(segment.face_vertex_counts.as_slice())
        } else {
            GeometryTopologyAuthoring::TriangleList
        };
        primitives.try_push_reserved(GeometryPrimitiveAuthoring::new(
            topology,
            Presence::Present(segment.indices.as_slice()),
            segment.material.map_or(Presence::Absent, Presence::Present),
            attribute_range,
        ))?;
        let mut logical_mesh = GeometryLogicalMeshAuthoring::new(AuthoringRange::new(local, 1));
        if let Some(name) = segment
            .key
            .display_name()
            .map(|id| state_name(scope, parsed, id))
            .transpose()?
            .and_then(bounded_entity_name)
        {
            logical_mesh = logical_mesh.with_name(name);
        }
        logical_meshes.try_push_reserved(logical_mesh)?;
        mesh_instances.try_push_reserved(GeometryMeshInstanceAuthoring::new(
            node_id,
            LogicalMeshLocalId::new(local),
        ))?;
        logical_morphs
            .try_push_reserved(LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[]))?;
        primitive_deformations
            .try_push_reserved(PrimitiveDeformationAuthoring::new(Presence::Absent, &[]))?;
        mesh_instance_deformations.try_push_reserved(MeshInstanceDeformationAuthoring::new(
            Presence::Absent,
            Presence::Absent,
        ))?;
    }

    let texture_count = materials
        .as_slice()
        .iter()
        .filter(|material| material.diffuse_texture.is_some())
        .count();
    let mut images = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-images",
    );
    let mut samplers = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-samplers",
    );
    let mut textures = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-textures",
    );
    let mut material_bindings = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-material-bindings",
    );
    let mut material_authoring = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-material-authoring",
    );
    images.try_reserve(texture_count)?;
    samplers.try_reserve(texture_count)?;
    textures.try_reserve(texture_count)?;
    material_bindings.try_reserve(texture_count)?;
    material_authoring.try_reserve(materials.len())?;

    let texture_role = TextureRole::Diffuse;
    let coordinate_source = TextureCoordinateSource::uv_set(0)
        .map_err(|_| scope.format_failure(ObjFailure::internal("invalid-texture-uv-set")))?;
    let channel_mapping = TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA);
    let color_interpretation = ColorInterpretation::SrgbColor;
    let blend = TextureBlendOperation::Replace;
    let source_key = TextureSourceKey::try_new("map_Kd")
        .map_err(|_| scope.format_failure(ObjFailure::internal("invalid-texture-source-key")))?;

    for material in materials.as_slice() {
        let binding_offset = canonical_u32(
            scope,
            material_bindings.len(),
            "obj-material-binding-offset",
        )?;
        let binding_count = if material.diffuse_texture.is_some() {
            let image_index = canonical_u32(scope, images.len(), "obj-image-local-id")?;
            let sampler_index = canonical_u32(scope, samplers.len(), "obj-sampler-local-id")?;
            let texture_index = canonical_u32(scope, textures.len(), "obj-texture-local-id")?;
            let resource_index = image_index
                .checked_add(1)
                .ok_or_else(|| scope.allocation_failure("obj-texture-resource-local-id"))?;
            images.try_push_reserved(ImageAuthoring::session_resource(
                ResourceIdentityLocalId::new(resource_index),
                Presence::Absent,
            ))?;
            samplers.try_push_reserved(SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT))?;
            textures.try_push_reserved(TextureAuthoring::new(
                ImageLocalId::new(image_index),
                SamplerLocalId::new(sampler_index),
            ))?;
            material_bindings.try_push_reserved(
                MaterialBindingAuthoring::try_from_parts(
                    TextureLocalId::new(texture_index),
                    &texture_role,
                    &coordinate_source,
                    &channel_mapping,
                    &color_interpretation,
                    TextureTransform::IDENTITY,
                    TextureStrength::ONE,
                    &blend,
                    Some(&source_key),
                )
                .map_err(|_| {
                    scope.format_failure(ObjFailure::internal("invalid-texture-binding"))
                })?,
            )?;
            1
        } else {
            0
        };
        let authored = MaterialAuthoring::try_from_parts(
            AuthoringRange::new(binding_offset, binding_count),
            &material.shading_model,
            &material.alpha_mode,
            false,
            material.factors,
        )
        .map_err(|_| scope.format_failure(ObjFailure::internal("invalid-material-authoring")))?
        .with_name(material.name);
        material_authoring.try_push_reserved(authored)?;
    }

    let mut pending_metadata = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-pending-metadata",
    );
    for (index, material) in materials.as_slice().iter().enumerate() {
        if let Some(metadata) = material.metadata {
            append_material_metadata(scope, &mut pending_metadata, index, metadata)?;
        }
    }
    for (index, segment) in segments.as_slice().iter().enumerate() {
        append_segment_metadata(scope, &mut pending_metadata, parsed, index, segment.key)?;
    }
    let mut metadata_entries = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-metadata-authoring",
    );
    metadata_entries.try_reserve(pending_metadata.len())?;
    for metadata in pending_metadata.as_slice() {
        metadata_entries.try_push_reserved(MetadataAuthoringEntry::new(
            metadata.owner,
            &metadata.key,
            metadata.value.as_extension(),
        ))?;
    }

    let source_provenance = [ProvenanceLocalId::new(0)];
    let (resource_identity, resource_stability) = profile_resource_identity(scope)?;
    let format =
        FormatId::new(FORMAT_ID).map_err(|_| scope.allocation_failure("obj-profile-format-id"))?;
    let resource_count = texture_count
        .checked_add(1)
        .ok_or_else(|| scope.allocation_failure("obj-profile-resource-count"))?;
    let mut resources = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-resources",
    );
    let mut resource_snapshots = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-resource-snapshots",
    );
    let mut source_locators = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-source-locators",
    );
    resources.try_reserve(resource_count)?;
    resource_snapshots.try_reserve(resource_count)?;
    source_locators.try_reserve(resource_count)?;
    resources.try_push_reserved(ObservedResourceAuthoring::session_opaque(
        &resource_identity,
        resource_stability,
    ))?;
    resource_snapshots.try_push_reserved(ResourceSnapshotAuthoring::absent())?;
    source_locators
        .try_push_reserved(SourceObjectLocator::indexed(SourceObjectKind::Document, 0))?;
    for (image_index, texture) in materials
        .as_slice()
        .iter()
        .filter_map(|material| material.diffuse_texture.as_ref())
        .enumerate()
    {
        resources.try_push_reserved(ObservedResourceAuthoring::session_opaque(
            &texture.identity,
            SessionOpaqueReason::UnregisteredResolver,
        ))?;
        resource_snapshots.try_push_reserved(ResourceSnapshotAuthoring::snapshotted(
            texture.snapshot.bytes(),
        ))?;
        source_locators.try_push_reserved(SourceObjectLocator::indexed(
            SourceObjectKind::Image,
            u64::try_from(image_index)
                .map_err(|_| scope.allocation_failure("obj-profile-image-locator"))?,
        ))?;
    }

    let mut source_identities = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-source-identities",
    );
    let mut provenance_nodes = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-provenance-nodes",
    );
    source_identities.try_reserve(resource_count)?;
    provenance_nodes.try_reserve(resource_count)?;
    for (index, locator) in source_locators.as_slice().iter().enumerate() {
        let local = canonical_u32(scope, index, "obj-profile-source-identity")?;
        source_identities.try_push_reserved(SourceIdentityAuthoring::new(
            ResourceIdentityLocalId::new(local),
            &format,
            locator,
        ))?;
        provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::source(
            SourceIdentityLocalId::new(local),
        ))?;
    }

    let appearance_owner_count = texture_count
        .checked_mul(4)
        .ok_or_else(|| scope.allocation_failure("obj-profile-appearance-owner-count"))?;
    let owner_count = 2usize
        .checked_add(nodes.len())
        .and_then(|count| count.checked_add(appearance_owner_count))
        .and_then(|count| count.checked_add(material_authoring.len()))
        .and_then(|count| count.checked_add(logical_meshes.len()))
        .and_then(|count| count.checked_add(primitives.len()))
        .and_then(|count| count.checked_add(attributes.len()))
        .and_then(|count| count.checked_add(mesh_instances.len()))
        .ok_or_else(|| scope.allocation_failure("obj-profile-owner-count"))?;
    let mut appearance_provenance = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-appearance-provenance",
    );
    appearance_provenance.try_reserve(texture_count)?;
    for index in 0..texture_count {
        let resource = canonical_u32(
            scope,
            index
                .checked_add(1)
                .ok_or_else(|| scope.allocation_failure("obj-texture-provenance"))?,
            "obj-texture-provenance",
        )?;
        appearance_provenance.try_push_reserved(ProvenanceLocalId::new(resource))?;
    }
    let mut owner_provenance = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-owner-provenance",
    );
    owner_provenance.try_reserve(owner_count)?;
    owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
        ProfileOwner::Scene,
        &source_provenance,
    ))?;
    owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
        ProfileOwner::SyntheticRoot,
        &source_provenance,
    ))?;
    for index in 0..nodes.len() {
        owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
            ProfileOwner::Node(NodeLocalId::new(canonical_u32(
                scope,
                index,
                "obj-profile-node-owner",
            )?)),
            &source_provenance,
        ))?;
    }
    for index in 0..texture_count {
        let local = canonical_u32(scope, index, "obj-profile-appearance-owner")?;
        let provenance = &appearance_provenance.as_slice()[index..=index];
        for owner in [
            ProfileOwner::Image(local),
            ProfileOwner::Sampler(local),
            ProfileOwner::Texture(local),
            ProfileOwner::TextureBinding(local),
        ] {
            owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(owner, provenance))?;
        }
    }
    push_indexed_owners(
        scope,
        &mut owner_provenance,
        material_authoring.len(),
        "obj-profile-material-owner",
        ProfileOwner::Material,
        &source_provenance,
    )?;
    push_indexed_owners(
        scope,
        &mut owner_provenance,
        logical_meshes.len(),
        "obj-profile-logical-mesh-owner",
        ProfileOwner::LogicalMesh,
        &source_provenance,
    )?;
    push_indexed_owners(
        scope,
        &mut owner_provenance,
        primitives.len(),
        "obj-profile-primitive-owner",
        ProfileOwner::Primitive,
        &source_provenance,
    )?;
    push_indexed_owners(
        scope,
        &mut owner_provenance,
        attributes.len(),
        "obj-profile-attribute-owner",
        ProfileOwner::Attribute,
        &source_provenance,
    )?;
    push_indexed_owners(
        scope,
        &mut owner_provenance,
        mesh_instances.len(),
        "obj-profile-mesh-instance-owner",
        ProfileOwner::MeshInstance,
        &source_provenance,
    )?;

    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let node_batch = NodeAuthoringBatch::new(Presence::Present(roots.as_slice()), nodes.as_slice());
    let appearance = AppearanceAuthoringBatch::new(
        images.as_slice(),
        samplers.as_slice(),
        textures.as_slice(),
        material_bindings.as_slice(),
        material_authoring.as_slice(),
    );
    let geometry = GeometryAuthoringBatch::new(
        logical_meshes.as_slice(),
        primitives.as_slice(),
        attributes.as_slice(),
        mesh_instances.as_slice(),
    );
    let deformation = DeformationAuthoringBatch::new(
        &[],
        &[],
        logical_morphs.as_slice(),
        primitive_deformations.as_slice(),
        mesh_instance_deformations.as_slice(),
    );
    let animations = AnimationAuthoringBatch::empty();
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(
            resources.as_slice(),
            source_identities.as_slice(),
            provenance_nodes.as_slice(),
            owner_provenance.as_slice(),
        ),
        MetadataAuthoringBatch::new(metadata_entries.as_slice()),
    )
    .with_resource_snapshots(resource_snapshots.as_slice());
    let input = SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile,
    );
    scope.submit_scene(input)
}

fn append_material_metadata<'scope, 'input>(
    scope: &ParseScope<'_>,
    output: &mut MeteredVec<'scope, PendingMetadata<'input>>,
    material: usize,
    metadata: &'input ParsedMetadata<'scope>,
) -> ParseResult<()> {
    let owner = ProfileOwner::Material(canonical_u32(
        scope,
        material,
        "obj-metadata-material-owner",
    )?);
    for (key, value) in metadata.iter() {
        let value = match value {
            ParsedMetadataValue::I64(value) => BorrowedMetadataValue::I64(*value),
            ParsedMetadataValue::F64(value) => BorrowedMetadataValue::F64(*value),
            ParsedMetadataValue::String(value) => BorrowedMetadataValue::String(value.as_str()),
        };
        push_metadata(scope, output, owner, key, value)?;
    }
    Ok(())
}

fn append_segment_metadata<'scope, 'input>(
    scope: &ParseScope<'_>,
    output: &mut MeteredVec<'scope, PendingMetadata<'input>>,
    parsed: &'input ParsedObj<'scope>,
    mesh: usize,
    key: SegmentKey,
) -> ParseResult<()> {
    let owner = ProfileOwner::LogicalMesh(canonical_u32(
        scope,
        mesh,
        "obj-metadata-logical-mesh-owner",
    )?);
    for (name, id) in [
        ("obj:group", key.group),
        ("obj:material", key.material),
        ("obj:object", key.object),
        ("obj:smoothing", key.smoothing),
    ] {
        if let Some(id) = id {
            push_metadata(
                scope,
                output,
                owner,
                name,
                BorrowedMetadataValue::String(state_name(scope, parsed, id)?),
            )?;
        }
    }
    Ok(())
}

fn push_metadata<'scope, 'input>(
    scope: &ParseScope<'_>,
    output: &mut MeteredVec<'scope, PendingMetadata<'input>>,
    owner: ProfileOwner,
    key: &str,
    value: BorrowedMetadataValue<'input>,
) -> ParseResult<()> {
    let key = NamespacedName::new(key)
        .map_err(|_| scope.format_failure(ObjFailure::internal("invalid-metadata-key")))?;
    output.try_push(PendingMetadata { owner, key, value })
}

fn push_indexed_owners<'scope, 'input>(
    scope: &ParseScope<'_>,
    output: &mut MeteredVec<'scope, OwnerProvenanceAuthoring<'input>>,
    count: usize,
    resource: &'static str,
    owner: impl Fn(u32) -> ProfileOwner,
    provenance: &'input [ProvenanceLocalId],
) -> ParseResult<()> {
    for index in 0..count {
        output.try_push_reserved(OwnerProvenanceAuthoring::new(
            owner(canonical_u32(scope, index, resource)?),
            provenance,
        ))?;
    }
    Ok(())
}

fn canonical_u32(scope: &ParseScope<'_>, value: usize, resource: &'static str) -> ParseResult<u32> {
    u32::try_from(value).map_err(|_| scope.allocation_failure(resource))
}

fn profile_resource_identity(
    scope: &ParseScope<'_>,
) -> ParseResult<(ResourceIdentity, SessionOpaqueReason)> {
    match ResourceIdentity::new(scope.source().as_str()) {
        Ok(identity) => Ok((identity, SessionOpaqueReason::CallerLabel)),
        Err(_) => ResourceIdentity::new(FALLBACK_RESOURCE_IDENTITY)
            .map(|identity| (identity, SessionOpaqueReason::SessionFallback))
            .map_err(|_| scope.allocation_failure("obj-profile-resource-identity")),
    }
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
