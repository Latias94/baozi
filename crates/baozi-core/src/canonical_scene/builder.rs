use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CanonicalSceneCapacityPlan {
    collection: CanonicalSceneCollection,
    original_len: usize,
    original_capacity: usize,
    replacement: Option<CapacityPlan>,
    pub(super) prepared_value_storage_bytes: u64,
}

impl CanonicalSceneCapacityPlan {
    pub(crate) fn collection_envelope_delta_bytes(self) -> Result<u64, CanonicalSceneBuildError> {
        self.replacement.map_or(Ok(0), |plan| {
            plan.envelope_delta_bytes()
                .map_err(|cause| CanonicalSceneBuildError::CapacityPlan {
                    collection: self.collection,
                    cause,
                })
        })
    }

    pub(crate) const fn value_storage_envelope_bytes(self) -> u64 {
        self.prepared_value_storage_bytes
    }

    pub(crate) const fn replaces_collection_allocation(self) -> bool {
        self.replacement.is_some()
    }
}

pub(crate) trait CanonicalPreparedSceneInsert {
    type Id;

    fn collection_retained_allocation_bytes(&self) -> u64;

    fn value_storage_retained_allocation_bytes(&self) -> u64;

    fn replaces_collection_allocation(&self) -> bool;

    fn commit(self) -> Self::Id;
}

struct PreparedCanonicalSceneInsert<'owner, T, Id> {
    owner: &'owner mut Vec<T>,
    replacement: Option<Vec<T>>,
    value: T,
    id: Id,
    collection_retained_allocation_bytes: u64,
    value_storage_retained_allocation_bytes: u64,
}

impl<T, Id> CanonicalPreparedSceneInsert for PreparedCanonicalSceneInsert<'_, T, Id> {
    type Id = Id;

    fn collection_retained_allocation_bytes(&self) -> u64 {
        self.collection_retained_allocation_bytes
    }

    fn value_storage_retained_allocation_bytes(&self) -> u64 {
        self.value_storage_retained_allocation_bytes
    }

    fn replaces_collection_allocation(&self) -> bool {
        self.replacement.is_some()
    }

    fn commit(self) -> Self::Id {
        let PreparedCanonicalSceneInsert {
            owner,
            replacement,
            value,
            id,
            collection_retained_allocation_bytes: _,
            value_storage_retained_allocation_bytes: _,
        } = self;
        if let Some(mut replacement) = replacement {
            assert!(
                replacement.capacity() > owner.len(),
                "a canonical replacement must admit every live value and the insertion"
            );
            replacement.append(owner);
            replacement.push(value);
            std::mem::swap(owner, &mut replacement);
            drop(replacement);
        } else {
            assert!(
                owner.len() < owner.capacity(),
                "an in-place canonical insertion requires an admitted slot"
            );
            owner.push(value);
        }
        id
    }
}

pub(crate) mod node_batch;
pub(crate) struct CanonicalSceneBuilder {
    pub(super) data: CanonicalSceneData,
}

impl CanonicalSceneBuilder {
    pub(crate) fn try_new(
        scene_space: SceneSpace,
        extension_schemas: ExtensionSchemaSnapshot,
        provenance_arena: ProvenanceArena,
        provenance: impl Into<ProvenanceAttachments>,
        absent_scene_bounds: DerivedAbsenceReason,
    ) -> Result<Self, CanonicalSceneBuildError> {
        let provenance = provenance.into();
        let provenance_retained_bytes = provenance.retained_allocation_bytes();
        let provenance = try_share_scene_payload(
            provenance,
            provenance_retained_bytes,
            CanonicalSceneSharedPayload::Provenance,
        )?;
        let extension_schema_retained_bytes = extension_schemas.retained_allocation_bytes();
        let extension_schemas = try_share_scene_payload(
            extension_schemas,
            extension_schema_retained_bytes,
            CanonicalSceneSharedPayload::ExtensionSchemas,
        )?;
        let metadata = try_share_scene_payload(
            SceneMetadata::empty(),
            0,
            CanonicalSceneSharedPayload::Metadata,
        )?;
        let provenance_arena =
            SharedStorage::try_new(provenance_arena).map_err(|error| match error {
                SharedStorageError::AllocationFailed => {
                    CanonicalSceneBuildError::ProvenanceArenaAllocationFailed
                }
                SharedStorageError::AccountingOverflow => {
                    CanonicalSceneBuildError::ProvenanceArenaAccountingOverflow
                }
            })?;
        Ok(Self {
            data: CanonicalSceneData {
                generation: SceneGenerationId::allocate()?,
                root: None,
                scene_space,
                provenance,
                nodes: Vec::new(),
                mesh_instances: Vec::new(),
                logical_meshes: Vec::new(),
                materials: Vec::new(),
                images: Vec::new(),
                samplers: Vec::new(),
                textures: Vec::new(),
                skins: Vec::new(),
                animation_clips: Vec::new(),
                cameras: Vec::new(),
                lights: Vec::new(),
                extension_schemas,
                metadata,
                provenance_arena,
                scene_bounds: SceneBounds::absent(absent_scene_bounds),
            },
        })
    }

    pub(crate) const fn generation(&self) -> SceneGenerationId {
        self.data.generation
    }

    pub(crate) fn collection_capacity_plan(
        &self,
        collection: CanonicalSceneCollection,
    ) -> Result<CanonicalSceneCapacityPlan, CanonicalSceneBuildError> {
        match collection {
            CanonicalSceneCollection::Nodes => {
                canonical_capacity_plan(&self.data.nodes, collection, 0)
            }
            CanonicalSceneCollection::MeshInstances => canonical_capacity_plan(
                &self.data.mesh_instances,
                collection,
                shared_value_layout_bytes::<MeshInstance>(collection)?,
            ),
            CanonicalSceneCollection::LogicalMeshes => {
                canonical_capacity_plan(&self.data.logical_meshes, collection, 0)
            }
            CanonicalSceneCollection::Materials => canonical_capacity_plan(
                &self.data.materials,
                collection,
                shared_value_layout_bytes::<Material>(collection)?,
            ),
            CanonicalSceneCollection::Images => canonical_capacity_plan(
                &self.data.images,
                collection,
                shared_value_layout_bytes::<Image>(collection)?,
            ),
            CanonicalSceneCollection::Samplers => canonical_capacity_plan(
                &self.data.samplers,
                collection,
                shared_value_layout_bytes::<Sampler>(collection)?,
            ),
            CanonicalSceneCollection::Textures => canonical_capacity_plan(
                &self.data.textures,
                collection,
                shared_value_layout_bytes::<Texture>(collection)?,
            ),
            CanonicalSceneCollection::Skins => canonical_capacity_plan(
                &self.data.skins,
                collection,
                shared_value_layout_bytes::<Skin>(collection)?,
            ),
            CanonicalSceneCollection::AnimationClips => canonical_capacity_plan(
                &self.data.animation_clips,
                collection,
                shared_value_layout_bytes::<AnimationClip>(collection)?,
            ),
            CanonicalSceneCollection::Cameras => canonical_capacity_plan(
                &self.data.cameras,
                collection,
                shared_value_layout_bytes::<Camera>(collection)?,
            ),
            CanonicalSceneCollection::Lights => canonical_capacity_plan(
                &self.data.lights,
                collection,
                shared_value_layout_bytes::<Light>(collection)?,
            ),
        }
    }

    pub(crate) fn try_push_node(
        &mut self,
        node: Node,
    ) -> Result<NodeIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::Nodes)?;
        self.prepare_push_node(node, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_node(
        &mut self,
        node: Node,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<impl CanonicalPreparedSceneInsert<Id = NodeIndex> + '_, CanonicalSceneBuildError>
    {
        let index = NodeIndex::try_from(self.data.nodes.len())?;
        prepare_owned_insert(
            &mut self.data.nodes,
            node,
            index,
            plan,
            CanonicalSceneCollection::Nodes,
        )
    }

    pub(crate) fn try_push_mesh_instance(
        &mut self,
        instance: MeshInstance,
    ) -> Result<MeshInstanceIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::MeshInstances)?;
        self.prepare_push_mesh_instance(instance, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_mesh_instance(
        &mut self,
        instance: MeshInstance,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<
        impl CanonicalPreparedSceneInsert<Id = MeshInstanceIndex> + '_,
        CanonicalSceneBuildError,
    > {
        let index = MeshInstanceIndex::try_from(self.data.mesh_instances.len())?;
        prepare_shared_insert(
            &mut self.data.mesh_instances,
            instance,
            index,
            plan,
            CanonicalSceneCollection::MeshInstances,
        )
    }

    pub(crate) fn try_push_logical_mesh(
        &mut self,
        mesh: LogicalMesh,
    ) -> Result<LogicalMeshIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::LogicalMeshes)?;
        self.prepare_push_logical_mesh(mesh, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_logical_mesh(
        &mut self,
        mesh: LogicalMesh,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<
        impl CanonicalPreparedSceneInsert<Id = LogicalMeshIndex> + '_,
        CanonicalSceneBuildError,
    > {
        let index = LogicalMeshIndex::try_from(self.data.logical_meshes.len())?;
        prepare_owned_insert(
            &mut self.data.logical_meshes,
            mesh,
            index,
            plan,
            CanonicalSceneCollection::LogicalMeshes,
        )
    }

    pub(crate) fn try_push_material(
        &mut self,
        material: Material,
    ) -> Result<MaterialIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::Materials)?;
        self.prepare_push_material(material, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_material(
        &mut self,
        material: Material,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<impl CanonicalPreparedSceneInsert<Id = MaterialIndex> + '_, CanonicalSceneBuildError>
    {
        let index = MaterialIndex::try_from(self.data.materials.len())?;
        prepare_shared_insert(
            &mut self.data.materials,
            material,
            index,
            plan,
            CanonicalSceneCollection::Materials,
        )
    }

    pub(crate) fn try_push_image(
        &mut self,
        image: Image,
    ) -> Result<ImageIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::Images)?;
        self.prepare_push_image(image, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_image(
        &mut self,
        image: Image,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<impl CanonicalPreparedSceneInsert<Id = ImageIndex> + '_, CanonicalSceneBuildError>
    {
        let index = ImageIndex::try_from(self.data.images.len())?;
        prepare_shared_insert(
            &mut self.data.images,
            image,
            index,
            plan,
            CanonicalSceneCollection::Images,
        )
    }

    pub(crate) fn try_push_sampler(
        &mut self,
        sampler: Sampler,
    ) -> Result<SamplerIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::Samplers)?;
        self.prepare_push_sampler(sampler, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_sampler(
        &mut self,
        sampler: Sampler,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<impl CanonicalPreparedSceneInsert<Id = SamplerIndex> + '_, CanonicalSceneBuildError>
    {
        let index = SamplerIndex::try_from(self.data.samplers.len())?;
        prepare_shared_insert(
            &mut self.data.samplers,
            sampler,
            index,
            plan,
            CanonicalSceneCollection::Samplers,
        )
    }

    pub(crate) fn try_push_texture(
        &mut self,
        texture: Texture,
    ) -> Result<TextureIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::Textures)?;
        self.prepare_push_texture(texture, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_texture(
        &mut self,
        texture: Texture,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<impl CanonicalPreparedSceneInsert<Id = TextureIndex> + '_, CanonicalSceneBuildError>
    {
        let index = TextureIndex::try_from(self.data.textures.len())?;
        prepare_shared_insert(
            &mut self.data.textures,
            texture,
            index,
            plan,
            CanonicalSceneCollection::Textures,
        )
    }

    pub(crate) fn try_push_skin(
        &mut self,
        skin: crate::deform::Skin,
    ) -> Result<SkinIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::Skins)?;
        self.prepare_push_skin(skin, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_skin(
        &mut self,
        skin: Skin,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<impl CanonicalPreparedSceneInsert<Id = SkinIndex> + '_, CanonicalSceneBuildError>
    {
        let index = SkinIndex::try_from(self.data.skins.len())?;
        prepare_shared_insert(
            &mut self.data.skins,
            skin,
            index,
            plan,
            CanonicalSceneCollection::Skins,
        )
    }

    pub(crate) fn try_push_animation_clip(
        &mut self,
        clip: AnimationClip,
    ) -> Result<AnimationClipIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::AnimationClips)?;
        self.prepare_push_animation_clip(clip, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_animation_clip(
        &mut self,
        clip: AnimationClip,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<
        impl CanonicalPreparedSceneInsert<Id = AnimationClipIndex> + '_,
        CanonicalSceneBuildError,
    > {
        let index = AnimationClipIndex::try_from(self.data.animation_clips.len())?;
        prepare_shared_insert(
            &mut self.data.animation_clips,
            clip,
            index,
            plan,
            CanonicalSceneCollection::AnimationClips,
        )
    }

    pub(crate) fn try_push_camera(
        &mut self,
        camera: Camera,
    ) -> Result<CameraIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::Cameras)?;
        self.prepare_push_camera(camera, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_camera(
        &mut self,
        camera: Camera,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<impl CanonicalPreparedSceneInsert<Id = CameraIndex> + '_, CanonicalSceneBuildError>
    {
        let index = CameraIndex::try_from(self.data.cameras.len())?;
        prepare_shared_insert(
            &mut self.data.cameras,
            camera,
            index,
            plan,
            CanonicalSceneCollection::Cameras,
        )
    }

    pub(crate) fn try_push_light(
        &mut self,
        light: Light,
    ) -> Result<LightIndex, CanonicalSceneBuildError> {
        let plan = self.collection_capacity_plan(CanonicalSceneCollection::Lights)?;
        self.prepare_push_light(light, plan)
            .map(CanonicalPreparedSceneInsert::commit)
    }

    pub(crate) fn prepare_push_light(
        &mut self,
        light: Light,
        plan: CanonicalSceneCapacityPlan,
    ) -> Result<impl CanonicalPreparedSceneInsert<Id = LightIndex> + '_, CanonicalSceneBuildError>
    {
        let index = LightIndex::try_from(self.data.lights.len())?;
        prepare_shared_insert(
            &mut self.data.lights,
            light,
            index,
            plan,
            CanonicalSceneCollection::Lights,
        )
    }

    pub(crate) fn set_metadata(
        &mut self,
        metadata: SceneMetadata,
    ) -> Result<(), CanonicalSceneBuildError> {
        let retained_bytes = metadata.retained_allocation_bytes().map_err(|_| {
            CanonicalSceneBuildError::SharedPayloadAccountingOverflow {
                payload: CanonicalSceneSharedPayload::Metadata,
            }
        })?;
        self.data.metadata = try_share_scene_payload(
            metadata,
            retained_bytes,
            CanonicalSceneSharedPayload::Metadata,
        )?;
        Ok(())
    }

    pub(crate) fn set_scene_bounds(&mut self, bounds: SceneBounds) {
        self.data.scene_bounds = bounds;
    }

    pub(crate) fn set_root(&mut self, root: NodeIndex) {
        self.data.root = Some(root);
    }

    pub(crate) fn finish_draft(self) -> CanonicalSceneDraft {
        CanonicalSceneDraft { data: self.data }
    }
}

fn shared_value_layout_bytes<T>(
    collection: CanonicalSceneCollection,
) -> Result<u64, CanonicalSceneBuildError> {
    SharedStorage::<T>::checked_layout_footprint()
        .ok()
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(CanonicalSceneBuildError::CollectionSizeOverflow { collection })
}

fn canonical_capacity_plan<T>(
    values: &Vec<T>,
    collection: CanonicalSceneCollection,
    prepared_value_storage_bytes: u64,
) -> Result<CanonicalSceneCapacityPlan, CanonicalSceneBuildError> {
    let required_capacity = values
        .len()
        .checked_add(1)
        .ok_or(CanonicalSceneBuildError::CollectionSizeOverflow { collection })?;
    let replacement = if required_capacity <= values.capacity() {
        None
    } else {
        Some(
            CapacityPlan::for_type::<T>(0, 0, required_capacity)
                .map_err(|cause| CanonicalSceneBuildError::CapacityPlan { collection, cause })?,
        )
    };
    Ok(CanonicalSceneCapacityPlan {
        collection,
        original_len: values.len(),
        original_capacity: values.capacity(),
        replacement,
        prepared_value_storage_bytes,
    })
}

fn allocate_collection_replacement<T>(
    values: &mut Vec<T>,
    plan: CanonicalSceneCapacityPlan,
    collection: CanonicalSceneCollection,
) -> Result<(Option<Vec<T>>, u64), CanonicalSceneBuildError> {
    if plan.collection != collection {
        return Err(CanonicalSceneBuildError::CapacityPlanCollectionMismatch {
            expected: collection,
            actual: plan.collection,
        });
    }
    if values.len() != plan.original_len || values.capacity() != plan.original_capacity {
        return Err(CanonicalSceneBuildError::CapacityPlan {
            collection,
            cause: CapacityPlanError::StalePlan {
                expected_len: plan.original_len,
                expected_capacity: plan.original_capacity,
                actual_len: values.len(),
                actual_capacity: values.capacity(),
            },
        });
    }
    let required_capacity = values
        .len()
        .checked_add(1)
        .ok_or(CanonicalSceneBuildError::CollectionSizeOverflow { collection })?;
    let Some(replacement_plan) = plan.replacement else {
        if required_capacity > values.capacity() {
            return Err(CanonicalSceneBuildError::CapacityPlan {
                collection,
                cause: CapacityPlanError::InvalidState {
                    len: required_capacity,
                    capacity: values.capacity(),
                },
            });
        }
        return Ok((None, 0));
    };

    replacement_plan
        .verify_preflight(0, 0)
        .map_err(|cause| CanonicalSceneBuildError::CapacityPlan { collection, cause })?;
    let mut replacement = Vec::new();
    replacement
        .try_reserve_exact(replacement_plan.allocator_additional())
        .map_err(|_| CanonicalSceneBuildError::AllocationFailed { collection })?;
    let replacement_bytes = replacement_plan
        .finish(replacement.len(), replacement.capacity())
        .map_err(|cause| CanonicalSceneBuildError::CapacityPlan { collection, cause })?;
    if replacement.capacity() < required_capacity {
        return Err(CanonicalSceneBuildError::CapacityPlan {
            collection,
            cause: CapacityPlanError::InvalidState {
                len: required_capacity,
                capacity: replacement.capacity(),
            },
        });
    }
    Ok((Some(replacement), replacement_bytes))
}

fn prepare_owned_insert<'owner, T, Id>(
    values: &'owner mut Vec<T>,
    value: T,
    id: Id,
    plan: CanonicalSceneCapacityPlan,
    collection: CanonicalSceneCollection,
) -> Result<PreparedCanonicalSceneInsert<'owner, T, Id>, CanonicalSceneBuildError> {
    prepare_insert(values, value, id, plan, collection, |value| Ok((value, 0)))
}

fn prepare_shared_insert<'owner, T, Id>(
    values: &'owner mut Vec<SharedStorage<T>>,
    value: T,
    id: Id,
    plan: CanonicalSceneCapacityPlan,
    collection: CanonicalSceneCollection,
) -> Result<PreparedCanonicalSceneInsert<'owner, SharedStorage<T>, Id>, CanonicalSceneBuildError> {
    prepare_insert(values, value, id, plan, collection, |value| {
        let value = SharedStorage::try_new(value).map_err(|error| match error {
            SharedStorageError::AllocationFailed => {
                CanonicalSceneBuildError::AllocationFailed { collection }
            }
            SharedStorageError::AccountingOverflow => {
                CanonicalSceneBuildError::CollectionSizeOverflow { collection }
            }
        })?;
        let value_bytes = u64::try_from(value.retained_allocation_bytes())
            .map_err(|_| CanonicalSceneBuildError::CollectionSizeOverflow { collection })?;
        Ok((value, value_bytes))
    })
}

fn prepare_insert<'owner, Source, Stored, Id>(
    values: &'owner mut Vec<Stored>,
    value: Source,
    id: Id,
    plan: CanonicalSceneCapacityPlan,
    collection: CanonicalSceneCollection,
    prepare_value: impl FnOnce(Source) -> Result<(Stored, u64), CanonicalSceneBuildError>,
) -> Result<PreparedCanonicalSceneInsert<'owner, Stored, Id>, CanonicalSceneBuildError> {
    let collection_admitted = plan.collection_envelope_delta_bytes()?;
    let value_storage_admitted = plan.value_storage_envelope_bytes();
    let (replacement, collection_retained_allocation_bytes) =
        allocate_collection_replacement(values, plan, collection)?;
    if collection_retained_allocation_bytes > collection_admitted {
        return Err(CanonicalSceneBuildError::PreparedAllocationUnderestimated {
            collection,
            allocation: CanonicalPreparedAllocation::Collection,
            admitted: collection_admitted,
            actual: collection_retained_allocation_bytes,
        });
    }
    let (value, value_storage_retained_allocation_bytes) = prepare_value(value)?;
    if value_storage_retained_allocation_bytes > value_storage_admitted {
        return Err(CanonicalSceneBuildError::PreparedAllocationUnderestimated {
            collection,
            allocation: CanonicalPreparedAllocation::ValueStorage,
            admitted: value_storage_admitted,
            actual: value_storage_retained_allocation_bytes,
        });
    }
    Ok(PreparedCanonicalSceneInsert {
        owner: values,
        replacement,
        value,
        id,
        collection_retained_allocation_bytes,
        value_storage_retained_allocation_bytes,
    })
}

fn try_share_scene_payload<T>(
    value: T,
    nested_allocation_bytes: usize,
    payload: CanonicalSceneSharedPayload,
) -> Result<SharedStorage<T>, CanonicalSceneBuildError> {
    SharedStorage::try_new_prepared(value, nested_allocation_bytes).map_err(|error| match error {
        SharedStorageError::AllocationFailed => {
            CanonicalSceneBuildError::SharedPayloadAllocationFailed { payload }
        }
        SharedStorageError::AccountingOverflow => {
            CanonicalSceneBuildError::SharedPayloadAccountingOverflow { payload }
        }
    })
}
