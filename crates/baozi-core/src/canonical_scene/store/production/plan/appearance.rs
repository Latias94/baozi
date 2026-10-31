#![allow(
    clippy::result_large_err,
    reason = "appearance authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

//! Source validation and resolved image borrows for the typed appearance owner.
//! Binding/material semantics and ranges remain in the original immutable input.

use std::error::Error;
use std::fmt;

use baozi_operation::{CapacityPlan, CapacityPlanError};
use thiserror::Error;

use crate::appearance::{
    AppearanceBuildError, MaterialFactors, MaterialTextureBindingSemanticsValidator,
    validate_material_semantics, validate_texture_binding_semantics,
};
use crate::canonical_scene::store::typed_storage::appearance::build::AppearanceStoragePlan;
use crate::entity::validate_entity_name_str;
use crate::image::MAX_ENCODED_IMAGE_BYTES;
use crate::{SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits, SceneValidationWork};

use super::super::control::CanonicalAuthoringControl;
use super::super::input::appearance::{
    AppearanceAuthoringBatch, ImageAuthoring, ImagePayloadAuthoring, MaterialAuthoring,
    MaterialBindingAuthoring, SamplerAuthoring, TextureAuthoring,
};
use super::super::input::{Presence, ResourceSnapshotAuthoringTable};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum AppearanceAuthoringLimit {
    Entities,
    Edges,
    References,
    SemanticCells,
    CanonicalBytes,
    Work,
    ScratchBytes,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum AppearanceAuthoringSemanticError {
    #[error("appearance authoring count for {collection} exceeds the u32 canonical domain")]
    CountDomainTooLarge { collection: &'static str },
    #[error("{limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        limit: AppearanceAuthoringLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("appearance authoring accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error("image {image} has an empty encoded payload")]
    EmptyEncodedPayload { image: usize },
    #[error(
        "image {image} encoded payload has {actual} bytes; the canonical per-image maximum is {maximum}"
    )]
    EncodedPayloadTooLarge {
        image: usize,
        actual: usize,
        maximum: usize,
    },
    #[error(
        "session resource snapshot table has {snapshot_count} rows; the profile declares {resource_count} resources"
    )]
    ResourceSnapshotCardinalityMismatch {
        snapshot_count: usize,
        resource_count: usize,
    },
    #[error(
        "image {image} references session resource {resource}; the profile declares {resource_count} resources"
    )]
    ImageResourceOutOfRange {
        image: usize,
        resource: u32,
        resource_count: usize,
    },
    #[error("image {image} session resource {resource} has no snapshotted bytes")]
    ImageResourceSnapshotAbsent { image: usize, resource: u32 },
    #[error(
        "material {material} binding range starts at {actual}; canonical owner coverage expected {expected}"
    )]
    MaterialBindingRangeOrder {
        material: usize,
        expected: u32,
        actual: u32,
    },
    #[error("material {material} binding range exceeds binding count {binding_count}")]
    MaterialBindingRangeOutOfBounds {
        material: usize,
        binding_count: usize,
    },
    #[error("flat binding table has {unowned} rows without exactly one material owner")]
    UnownedBindings { unowned: usize },
    #[error("texture {texture} image {image} exceeds image count {image_count}")]
    TextureImageOutOfRange {
        texture: usize,
        image: u32,
        image_count: u32,
    },
    #[error("texture {texture} sampler {sampler} exceeds sampler count {sampler_count}")]
    TextureSamplerOutOfRange {
        texture: usize,
        sampler: u32,
        sampler_count: u32,
    },
    #[error("binding {binding} texture {texture} exceeds texture count {texture_count}")]
    BindingTextureOutOfRange {
        binding: usize,
        texture: u32,
        texture_count: u32,
    },
    #[error("binding {binding} has invalid canonical usage semantics")]
    InvalidBindingSemantics {
        binding: usize,
        #[source]
        source: AppearanceBuildError,
    },
    #[error("material {material} has invalid canonical material semantics")]
    InvalidMaterialSemantics {
        material: usize,
        #[source]
        source: AppearanceBuildError,
    },
    #[error("material {material} has an invalid entity name")]
    InvalidMaterialName {
        material: usize,
        #[source]
        source: crate::entity::EntityNameError,
    },
    #[error("material {material} has conflicting canonical binding authority")]
    InvalidMaterialBindingSemantics {
        material: usize,
        #[source]
        source: AppearanceBuildError,
    },
    #[error("scratch capacity planning failed for {resource}: {cause}")]
    ScratchCapacity {
        resource: &'static str,
        #[source]
        cause: CapacityPlanError,
    },
    #[error("scratch allocation failed for {resource}")]
    ScratchAllocation { resource: &'static str },
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum AppearanceAuthoringFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Semantic(#[from] AppearanceAuthoringSemanticError),
    #[error("appearance authoring validation control failed")]
    Control(#[source] E),
}

/// One image's source descriptor and payload resolved under the same session's
/// snapshot authority. No resource lookup or independently supplied bytes enter
/// the materializer after this validation.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct ValidatedAppearanceImage<'input> {
    input: ImageAuthoring<'input>,
    payload: &'input [u8],
}

impl<'input> ValidatedAppearanceImage<'input> {
    pub(in crate::canonical_scene::store) const fn payload(self) -> &'input [u8] {
        self.payload
    }

    pub(in crate::canonical_scene::store) const fn media_type(
        self,
    ) -> Presence<&'input crate::extension::ExtensionMediaType> {
        match self.input.payload() {
            ImagePayloadAuthoring::Encoded { media_type, .. }
            | ImagePayloadAuthoring::SessionResource { media_type, .. } => media_type,
        }
    }
}

#[must_use = "dropping the validated appearance batch releases its resolved-image reservation"]
pub(in crate::canonical_scene::store) struct ValidatedAppearanceBatch<'input, R> {
    compiled: AppearanceValidationStorage<'input, R>,
    input: AppearanceAuthoringBatch<'input>,
}

struct AppearanceValidationStorage<'input, R> {
    rows: [u32; 5],
    payloads: [u64; 2],
    entity_count: u64,
    edge_count: u64,
    reference_count: u64,
    semantic_cell_count: u64,
    canonical_bytes: u64,
    validation_work: u64,
    scratch_bytes: u64,
    // Only image resolution creates an input-sized plan. Every other field stays
    // in its original borrowed row, without parallel codec/offset authorities.
    image_payloads: Vec<&'input [u8]>,
    #[cfg(test)]
    storage_drop_probe: Option<AppearanceStorageDropProbe>,
    // Held solely to cover image_payloads; storage must drop first, including unwind.
    _reservation: R,
}

impl<'input, R> ValidatedAppearanceBatch<'input, R> {
    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn input(
        &self,
    ) -> AppearanceAuthoringBatch<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn image_count(&self) -> u32 {
        self.compiled.rows[0]
    }

    pub(in crate::canonical_scene::store) const fn sampler_count(&self) -> u32 {
        self.compiled.rows[1]
    }

    pub(in crate::canonical_scene::store) const fn texture_count(&self) -> u32 {
        self.compiled.rows[2]
    }

    pub(in crate::canonical_scene::store) const fn binding_count(&self) -> u32 {
        self.compiled.rows[3]
    }

    pub(in crate::canonical_scene::store) const fn material_count(&self) -> u32 {
        self.compiled.rows[4]
    }

    /// Five initialized row counts in AppearanceStoragePlan order.
    pub(in crate::canonical_scene::store) fn row_counts(&self) -> [u64; 5] {
        self.compiled.rows.map(u64::from)
    }

    /// Initialized image-byte and combined text-byte counts. No scalar pools exist.
    pub(in crate::canonical_scene::store) const fn payload_counts(&self) -> [u64; 2] {
        self.compiled.payloads
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn image_payload_byte_count(&self) -> u64 {
        self.compiled.payloads[0]
    }

    pub(in crate::canonical_scene::store) const fn text_byte_count(&self) -> u64 {
        self.compiled.payloads[1]
    }

    pub(in crate::canonical_scene::store) const fn entity_count(&self) -> u64 {
        self.compiled.entity_count
    }

    pub(in crate::canonical_scene::store) const fn edge_count(&self) -> u64 {
        self.compiled.edge_count
    }

    pub(in crate::canonical_scene::store) const fn reference_count(&self) -> u64 {
        self.compiled.reference_count
    }

    pub(in crate::canonical_scene::store) const fn semantic_cell_count(&self) -> u64 {
        self.compiled.semantic_cell_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(&self) -> u64 {
        self.compiled.canonical_bytes
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.compiled.validation_work
    }

    pub(in crate::canonical_scene::store) const fn scratch_bytes(&self) -> u64 {
        self.compiled.scratch_bytes
    }

    pub(in crate::canonical_scene::store) fn images(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedAppearanceImage<'input>> + '_ {
        self.input
            .images()
            .iter()
            .copied()
            .zip(self.compiled.image_payloads.iter().copied())
            .map(|(input, payload)| ValidatedAppearanceImage { input, payload })
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn image(
        &self,
        index: usize,
    ) -> Option<ValidatedAppearanceImage<'input>> {
        Some(ValidatedAppearanceImage {
            input: *self.input.images().get(index)?,
            payload: self.compiled.image_payloads.get(index).copied()?,
        })
    }

    pub(in crate::canonical_scene::store) const fn samplers(&self) -> &'input [SamplerAuthoring] {
        self.input.samplers()
    }

    pub(in crate::canonical_scene::store) const fn textures(&self) -> &'input [TextureAuthoring] {
        self.input.textures()
    }

    pub(in crate::canonical_scene::store) const fn bindings(
        &self,
    ) -> &'input [MaterialBindingAuthoring<'input>] {
        self.input.bindings()
    }

    pub(in crate::canonical_scene::store) const fn materials(
        &self,
    ) -> &'input [MaterialAuthoring<'input>] {
        self.input.materials()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_storage_drop_probe(
        &mut self,
        stage: std::rc::Rc<std::cell::Cell<u8>>,
    ) {
        self.compiled.storage_drop_probe = Some(AppearanceStorageDropProbe(stage));
    }
}

#[cfg(test)]
struct AppearanceStorageDropProbe(std::rc::Rc<std::cell::Cell<u8>>);

#[cfg(test)]
impl Drop for AppearanceStorageDropProbe {
    fn drop(&mut self) {
        assert_eq!(
            self.0.replace(1),
            0,
            "resolved appearance storage must begin the coupled drop sequence"
        );
    }
}

impl<R> fmt::Debug for ValidatedAppearanceBatch<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedAppearanceBatch")
            .field("rows", &self.compiled.rows)
            .field("payloads", &self.compiled.payloads)
            .field("entity_count", &self.compiled.entity_count)
            .field("edge_count", &self.compiled.edge_count)
            .field("reference_count", &self.compiled.reference_count)
            .field("semantic_cell_count", &self.compiled.semantic_cell_count)
            .field("canonical_bytes", &self.compiled.canonical_bytes)
            .field("validation_work", &self.compiled.validation_work)
            .field("scratch_bytes", &self.compiled.scratch_bytes)
            .field("reservation", &"<held>")
            .finish()
    }
}

struct AppearanceValidationScratch<'input, R> {
    image_payloads: Vec<&'input [u8]>,
    reservation: R,
}

type AppearanceValidationResult<T, C> =
    Result<T, AppearanceAuthoringFailure<<C as CanonicalAuthoringControl>::Error>>;

struct AppearanceValidationContext<'control, C: CanonicalAuthoringControl> {
    control: &'control mut C,
    limits: SceneValidationLimits,
    rows: [u64; 5],
    payloads: [u64; 2],
    work: u64,
    work_since_poll: u64,
    edges: u64,
    references: u64,
    semantic_cells: u64,
    canonical_bytes: u64,
}

impl<'control, C: CanonicalAuthoringControl> AppearanceValidationContext<'control, C> {
    const fn new(limits: SceneValidationLimits, control: &'control mut C) -> Self {
        Self {
            control,
            limits,
            rows: [0; 5],
            payloads: [0; 2],
            work: 0,
            work_since_poll: 0,
            edges: 0,
            references: 0,
            semantic_cells: 0,
            canonical_bytes: 0,
        }
    }

    fn poll_now(&mut self) -> AppearanceValidationResult<(), C> {
        self.control
            .poll()
            .map_err(AppearanceAuthoringFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn run_unit<R>(
        &mut self,
        kind: SceneValidationWork,
        action: impl FnMut() -> R,
    ) -> AppearanceValidationResult<R, C> {
        let final_work = self.work.checked_add(1).ok_or(
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work",
            },
        )?;
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::Work,
            final_work,
            self.limits.max_work(),
        )?;
        let result = self
            .control
            .run_unit(kind, action)
            .map_err(AppearanceAuthoringFailure::Control)?;
        self.work = final_work;
        self.work_since_poll += 1;
        if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
            self.poll_now()?;
        }
        Ok(result)
    }

    fn run_semantic_unit<T>(
        &mut self,
        kind: SceneValidationWork,
        action: impl FnMut() -> Result<T, AppearanceAuthoringSemanticError>,
    ) -> AppearanceValidationResult<T, C> {
        self.run_unit(kind, action)?
            .map_err(AppearanceAuthoringFailure::Semantic)
    }

    fn add_relations(&mut self, amount: u64) -> AppearanceValidationResult<(), C> {
        for _ in 0..amount {
            let current = self.edges;
            let maximum = self.limits.max_edges();
            self.edges = self.run_semantic_unit(SceneValidationWork::GraphEdge, || {
                next_limited(
                    current,
                    1,
                    maximum,
                    AppearanceAuthoringLimit::Edges,
                    "edges",
                )
            })?;
            let current = self.references;
            let maximum = self.limits.max_references();
            self.references = self.run_semantic_unit(SceneValidationWork::Reference, || {
                next_limited(
                    current,
                    1,
                    maximum,
                    AppearanceAuthoringLimit::References,
                    "references",
                )
            })?;
        }
        Ok(())
    }

    fn set_storage_shape(
        &mut self,
        rows: [u64; 5],
        payloads: [u64; 2],
    ) -> AppearanceValidationResult<(), C> {
        let plan = AppearanceStoragePlan::for_shape(rows, payloads).map_err(|_| {
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "typed appearance capacity shape",
            }
        })?;
        let bytes = plan.canonical_bytes().map_err(|_| {
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "typed appearance initialized footprint",
            }
        })?;
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::CanonicalBytes,
            bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.rows = rows;
        self.payloads = payloads;
        self.canonical_bytes = bytes;
        Ok(())
    }

    fn add_semantic_cells(&mut self, count: u64) -> AppearanceValidationResult<(), C> {
        self.semantic_cells = next_limited(
            self.semantic_cells,
            count,
            self.limits.max_payload_cells(),
            AppearanceAuthoringLimit::SemanticCells,
            "semantic cells",
        )?;
        Ok(())
    }

    fn add_pool_bytes(&mut self, index: usize, count: usize) -> AppearanceValidationResult<(), C> {
        let count = usize_to_u64(count, "appearance pool bytes")?;
        self.add_semantic_cells(count)?;
        let mut payloads = self.payloads;
        payloads[index] = payloads[index].checked_add(count).ok_or(
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "appearance pool bytes",
            },
        )?;
        self.set_storage_shape(self.rows, payloads)
    }

    fn reserve_scratch(&mut self, bytes: u64) -> AppearanceValidationResult<C::Reservation, C> {
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        let reservation = self
            .control
            .reserve_scratch(bytes)
            .map_err(AppearanceAuthoringFailure::Control)?;
        self.control
            .validate_scratch(&reservation)
            .map_err(AppearanceAuthoringFailure::Control)?;
        Ok(reservation)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> AppearanceValidationResult<(), C> {
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.control
            .resize_scratch(reservation, bytes)
            .map_err(AppearanceAuthoringFailure::Control)
    }
}

/// O(1): only top-level lengths determine the single resolved-image allocation.
/// Resource authority and image/text payloads are inspected after admission by
/// controlled source actions, never by this shape estimate.
pub(in crate::canonical_scene::store) fn appearance_shape_scratch_envelope(
    input: AppearanceAuthoringBatch<'_>,
) -> Result<u64, AppearanceAuthoringSemanticError> {
    let plan = image_payload_plan(input.images().len())?;
    plan.envelope_delta_bytes()
        .map_err(|cause| AppearanceAuthoringSemanticError::ScratchCapacity {
            resource: "resolved appearance image payloads",
            cause,
        })
}

pub(in crate::canonical_scene::store) fn validate_appearance_batch_with_control<'input, C>(
    input: AppearanceAuthoringBatch<'input>,
    resource_snapshots: ResourceSnapshotAuthoringTable<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedAppearanceBatch<'input, C::Reservation>, AppearanceAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    let mut context = AppearanceValidationContext::new(limits, control);
    context.poll_now()?;
    validate_resource_snapshot_table(resource_snapshots)?;

    let rows = [
        collection_count(input.images().len(), "images")?,
        collection_count(input.samplers().len(), "samplers")?,
        collection_count(input.textures().len(), "textures")?,
        collection_count(input.bindings().len(), "material bindings")?,
        collection_count(input.materials().len(), "materials")?,
    ];
    // Five u32 row domains sum to less than 2^35, so this u64 sum cannot overflow.
    let entity_count = rows.into_iter().map(u64::from).sum();
    enforce_limit::<C::Error>(
        AppearanceAuthoringLimit::Entities,
        entity_count,
        limits.max_entities(),
    )?;
    context.set_storage_shape(rows.map(u64::from), [0; 2])?;

    let plan = image_payload_plan(input.images().len())?;
    let envelope = appearance_shape_scratch_envelope(input)?;
    let reservation = context.reserve_scratch(envelope)?;
    let (image_payloads, scratch_actual) = allocate_image_payloads(plan)?;
    let mut storage = AppearanceValidationScratch {
        image_payloads,
        reservation,
    };
    context.resize_scratch(&mut storage.reservation, scratch_actual)?;

    for (image_index, image) in input.images().iter().copied().enumerate() {
        let (bytes, media) = context.run_semantic_unit(SceneValidationWork::Entity, || {
            let (bytes, media) = resolve_image_payload(image_index, image, resource_snapshots)?;
            if bytes.is_empty() {
                return Err(AppearanceAuthoringSemanticError::EmptyEncodedPayload {
                    image: image_index,
                });
            }
            validate_encoded_payload_limit(image_index, bytes.len())?;
            storage.image_payloads.push(bytes);
            Ok((bytes, media))
        })?;
        context.add_pool_bytes(0, bytes.len())?;
        // Encoded image contents are opaque here. Shape and byte admission use
        // the resolved slice length; actual copying and hashing meter their work.
        if let Presence::Present(media) = media {
            context.add_pool_bytes(1, media.as_str().len())?;
            for byte in media.as_str().bytes() {
                context.run_unit(SceneValidationWork::PayloadCell, || byte)?;
            }
        }
    }

    for sampler in input.samplers() {
        context.run_unit(SceneValidationWork::Entity, || sampler.state())?;
    }
    for (texture_index, texture) in input.textures().iter().copied().enumerate() {
        context.run_semantic_unit(SceneValidationWork::Entity, || {
            if texture.image().index() >= rows[0] {
                return Err(AppearanceAuthoringSemanticError::TextureImageOutOfRange {
                    texture: texture_index,
                    image: texture.image().index(),
                    image_count: rows[0],
                });
            }
            if texture.sampler().index() >= rows[1] {
                return Err(AppearanceAuthoringSemanticError::TextureSamplerOutOfRange {
                    texture: texture_index,
                    sampler: texture.sampler().index(),
                    sampler_count: rows[1],
                });
            }
            Ok(())
        })?;
        context.add_relations(2)?;
    }
    for (binding_index, binding) in input.bindings().iter().copied().enumerate() {
        let semantics = binding.semantics();
        context.run_semantic_unit(SceneValidationWork::Entity, || {
            if binding.texture().index() >= rows[2] {
                return Err(AppearanceAuthoringSemanticError::BindingTextureOutOfRange {
                    binding: binding_index,
                    texture: binding.texture().index(),
                    texture_count: rows[2],
                });
            }
            validate_texture_binding_semantics(semantics).map_err(|source| {
                AppearanceAuthoringSemanticError::InvalidBindingSemantics {
                    binding: binding_index,
                    source,
                }
            })
        })?;
        // Offset/scale/rotation and strength are six real inline numeric cells.
        // They are validated bounded domain values, not a retained scalar pool.
        context.add_semantic_cells(6)?;
        if let Some(key) = semantics.source_key() {
            context.add_pool_bytes(1, key.as_str().len())?;
            for byte in key.as_str().bytes() {
                context.run_unit(SceneValidationWork::PayloadCell, || byte)?;
            }
        }
        context.add_relations(1)?;
    }

    let mut expected_binding_offset = 0_u32;
    for (material_index, material) in input.materials().iter().copied().enumerate() {
        let range = material.bindings();
        let semantics = material.semantics();
        let (start, end) = context.run_semantic_unit(SceneValidationWork::Entity, || {
            validate_material_semantics(semantics).map_err(|source| {
                AppearanceAuthoringSemanticError::InvalidMaterialSemantics {
                    material: material_index,
                    source,
                }
            })?;
            if range.offset() != expected_binding_offset {
                return Err(
                    AppearanceAuthoringSemanticError::MaterialBindingRangeOrder {
                        material: material_index,
                        expected: expected_binding_offset,
                        actual: range.offset(),
                    },
                );
            }
            let end = range
                .offset()
                .checked_add(range.count())
                .filter(|&end| end <= rows[3])
                .ok_or(
                    AppearanceAuthoringSemanticError::MaterialBindingRangeOutOfBounds {
                        material: material_index,
                        binding_count: input.bindings().len(),
                    },
                )?;
            if let Presence::Present(name) = material.name() {
                validate_entity_name_str(name).map_err(|source| {
                    AppearanceAuthoringSemanticError::InvalidMaterialName {
                        material: material_index,
                        source,
                    }
                })?;
            }
            Ok((
                u32_to_usize(range.offset(), "material binding start")?,
                u32_to_usize(end, "material binding end")?,
            ))
        })?;
        let mut authority = MaterialTextureBindingSemanticsValidator::new(end - start);
        for binding in &input.bindings()[start..end] {
            context.run_semantic_unit(SceneValidationWork::Traversal, || {
                authority
                    .validate_next(binding.semantics())
                    .map_err(|source| {
                        AppearanceAuthoringSemanticError::InvalidMaterialBindingSemantics {
                            material: material_index,
                            source,
                        }
                    })
            })?;
        }
        let numeric_cells = material_factor_cells(semantics.factors())
            + u64::from(semantics.alpha_mode().mask_cutoff().is_some());
        context.add_semantic_cells(numeric_cells)?;
        if let Presence::Present(name) = material.name() {
            context.add_pool_bytes(1, name.len())?;
            for byte in name.bytes() {
                context.run_unit(SceneValidationWork::PayloadCell, || byte)?;
            }
        }
        context.add_relations(u64::from(range.count()))?;
        expected_binding_offset = u32::try_from(end).map_err(|_| {
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "material binding end",
            }
        })?;
    }
    context.run_semantic_unit(SceneValidationWork::Traversal, || {
        if expected_binding_offset == rows[3] {
            Ok(())
        } else {
            Err(AppearanceAuthoringSemanticError::UnownedBindings {
                unowned: u32_to_usize(rows[3] - expected_binding_offset, "unowned bindings")?,
            })
        }
    })?;
    context.poll_now()?;

    let AppearanceValidationScratch {
        image_payloads,
        reservation,
    } = storage;
    Ok(ValidatedAppearanceBatch {
        input,
        compiled: AppearanceValidationStorage {
            rows,
            payloads: context.payloads,
            entity_count,
            edge_count: context.edges,
            reference_count: context.references,
            semantic_cell_count: context.semantic_cells,
            canonical_bytes: context.canonical_bytes,
            validation_work: context.work,
            scratch_bytes: scratch_actual,
            image_payloads,
            #[cfg(test)]
            storage_drop_probe: None,
            _reservation: reservation,
        },
    })
}

/// Counts only authored numeric components. Absent optional groups have no
/// numeric payload; default supplemental factors and emissive are always present.
fn material_factor_cells(factors: MaterialFactors) -> u64 {
    3 + 8
        + u64::from(factors.pbr_metallic_roughness().is_some()) * 6
        + u64::from(factors.pbr_specular_glossiness().is_some()) * 8
        + u64::from(factors.legacy().is_some()) * 14
}

pub(in crate::canonical_scene::store) fn validate_encoded_payload_limit(
    image: usize,
    actual: usize,
) -> Result<(), AppearanceAuthoringSemanticError> {
    if actual <= MAX_ENCODED_IMAGE_BYTES {
        Ok(())
    } else {
        Err(AppearanceAuthoringSemanticError::EncodedPayloadTooLarge {
            image,
            actual,
            maximum: MAX_ENCODED_IMAGE_BYTES,
        })
    }
}

fn validate_resource_snapshot_table(
    resource_snapshots: ResourceSnapshotAuthoringTable<'_>,
) -> Result<(), AppearanceAuthoringSemanticError> {
    let snapshot_count = resource_snapshots.snapshots().len();
    let resource_count = resource_snapshots.resource_count();
    if snapshot_count == 0 || snapshot_count == resource_count {
        Ok(())
    } else {
        Err(
            AppearanceAuthoringSemanticError::ResourceSnapshotCardinalityMismatch {
                snapshot_count,
                resource_count,
            },
        )
    }
}

fn resolve_image_payload<'input>(
    image: usize,
    authored: ImageAuthoring<'input>,
    resource_snapshots: ResourceSnapshotAuthoringTable<'input>,
) -> Result<
    (
        &'input [u8],
        Presence<&'input crate::extension::ExtensionMediaType>,
    ),
    AppearanceAuthoringSemanticError,
> {
    match authored.payload() {
        ImagePayloadAuthoring::Encoded { bytes, media_type } => Ok((bytes, media_type)),
        ImagePayloadAuthoring::SessionResource {
            resource,
            media_type,
        } => {
            let index = u32_to_usize(resource.index(), "image session resource index")?;
            if index >= resource_snapshots.resource_count() {
                return Err(AppearanceAuthoringSemanticError::ImageResourceOutOfRange {
                    image,
                    resource: resource.index(),
                    resource_count: resource_snapshots.resource_count(),
                });
            }
            let bytes = resource_snapshots
                .snapshots()
                .get(index)
                .map(|snapshot| snapshot.bytes())
                .ok_or(
                    AppearanceAuthoringSemanticError::ImageResourceSnapshotAbsent {
                        image,
                        resource: resource.index(),
                    },
                )?;
            let Presence::Present(bytes) = bytes else {
                return Err(
                    AppearanceAuthoringSemanticError::ImageResourceSnapshotAbsent {
                        image,
                        resource: resource.index(),
                    },
                );
            };
            Ok((bytes, media_type))
        }
    }
}

fn image_payload_plan(count: usize) -> Result<CapacityPlan, AppearanceAuthoringSemanticError> {
    CapacityPlan::for_type::<&[u8]>(0, 0, count).map_err(|cause| {
        AppearanceAuthoringSemanticError::ScratchCapacity {
            resource: "resolved appearance image payloads",
            cause,
        }
    })
}

fn allocate_image_payloads<'input>(
    plan: CapacityPlan,
) -> Result<(Vec<&'input [u8]>, u64), AppearanceAuthoringSemanticError> {
    const RESOURCE: &str = "resolved appearance image payloads";
    plan.verify_preflight(0, 0).map_err(|cause| {
        AppearanceAuthoringSemanticError::ScratchCapacity {
            resource: RESOURCE,
            cause,
        }
    })?;
    let mut payloads = Vec::new();
    payloads
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| AppearanceAuthoringSemanticError::ScratchAllocation { resource: RESOURCE })?;
    let bytes = plan.finish(0, payloads.capacity()).map_err(|cause| {
        AppearanceAuthoringSemanticError::ScratchCapacity {
            resource: RESOURCE,
            cause,
        }
    })?;
    Ok((payloads, bytes))
}

fn next_limited(
    current: u64,
    amount: u64,
    maximum: u64,
    limit: AppearanceAuthoringLimit,
    dimension: &'static str,
) -> Result<u64, AppearanceAuthoringSemanticError> {
    let actual = current
        .checked_add(amount)
        .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow { dimension })?;
    if actual > maximum {
        Err(AppearanceAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    } else {
        Ok(actual)
    }
}

fn collection_count(
    count: usize,
    collection: &'static str,
) -> Result<u32, AppearanceAuthoringSemanticError> {
    u32::try_from(count)
        .map_err(|_| AppearanceAuthoringSemanticError::CountDomainTooLarge { collection })
}

fn usize_to_u64(
    value: usize,
    dimension: &'static str,
) -> Result<u64, AppearanceAuthoringSemanticError> {
    u64::try_from(value)
        .map_err(|_| AppearanceAuthoringSemanticError::AccountingOverflow { dimension })
}

fn u32_to_usize(
    value: u32,
    dimension: &'static str,
) -> Result<usize, AppearanceAuthoringSemanticError> {
    usize::try_from(value)
        .map_err(|_| AppearanceAuthoringSemanticError::AccountingOverflow { dimension })
}

fn enforce_limit<E>(
    limit: AppearanceAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), AppearanceAuthoringFailure<E>>
where
    E: Error + Send + Sync + 'static,
{
    if actual <= maximum {
        Ok(())
    } else {
        Err(AppearanceAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        }
        .into())
    }
}
