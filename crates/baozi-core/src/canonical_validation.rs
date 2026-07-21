use crate::animation::{AnimationSamplesView, AnimationTarget, AnimationValuesView};
use crate::appearance::AppearanceBehaviorKind;
use crate::attribute::{IntegerSignedness, Normalization, RegisteredPackedEncodingError};
use crate::canonical_scene::{
    CanonicalSceneData, CanonicalSceneDraft, CanonicalValidatedScene, SceneMetadataError,
};
use crate::deform::MorphDeltaSemantic;
use crate::derived::Derived;
use crate::derived_input::{
    ControlledDerivedInputStampError, DerivedInputStampError, DerivedInputStampEvidence,
    logical_mesh_bounds_input_stamp_controlled, primitive_bounds_input_stamp_controlled,
    scene_bounds_input_stamp_controlled,
};
use crate::entity::AuthoredTransformKind;
use crate::extension::{
    ControlledRetainedFootprintError, ExtensionBehaviorKind, ExtensionEntry,
    ExtensionEntryClassification, ExtensionSchema, ExtensionSchemaBehavior, ExtensionSchemaId,
    ExtensionSchemaOwner, ExtensionValueRef, ExtensionValueSchema,
};
use crate::geometry::{ControlledGeometryStorageError, GeometryRetainedFootprint};
use crate::identity::{SceneEntityKind, SceneGenerationId, SceneStorageIndex};
use crate::image::ImagePayloadView;
use crate::provenance::{
    ControlledProvenanceValidationError, Provenance, ProvenanceAttachments, ProvenanceNodeView,
    ProvenanceValidationIssue, validate_provenance_arena_controlled,
};
use crate::scene_space::SceneSpaceValidationError;
use crate::shared_storage::SharedStorage;
use crate::validation_control::{
    NoopSceneValidationControl, SCENE_VALIDATION_POLL_INTERVAL, SceneCanonicalCounts,
    SceneFootprint, SceneReservationKind, SceneValidationControl, SceneValidationLimits,
    SceneValidationStats, SceneValidationWork, try_publish_coupled, try_publish_coupled_with,
};
use baozi_operation::{CapacityPlan, GrowableCapacityPlan};
use std::error::Error;
use std::mem::size_of;
use thiserror::Error;

pub(super) struct CanonicalValidationProof {
    _private: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalValidationLimit {
    Nodes,
    Edges,
    Depth,
    Entities,
    References,
    PayloadCells,
    MetadataEntries,
    MetadataBytes,
    CanonicalBytes,
    ScratchBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalOwner {
    Scene,
    ProvenanceNode {
        index: usize,
    },
    Entity {
        kind: SceneEntityKind,
        index: usize,
    },
    Primitive {
        logical_mesh: usize,
        primitive: usize,
    },
    Attribute {
        owner: CanonicalAttributeOwner,
        attribute: usize,
    },
    MorphTargetDefinition {
        logical_mesh: usize,
        target: usize,
    },
    MorphDelta {
        logical_mesh: usize,
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
    },
    TextureBinding {
        material: usize,
        binding: usize,
    },
    ExtensionEntry {
        owner: CanonicalExtensionOwner,
        entry: usize,
    },
    AnimationChannel {
        clip: usize,
        channel: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalAttributeOwner {
    Primitive {
        logical_mesh: usize,
        primitive: usize,
    },
    MeshInstance {
        instance: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalExtensionOwner {
    SceneMetadata,
    Entity { kind: SceneEntityKind, index: usize },
    TextureBinding { material: usize, binding: usize },
    AnimationTimingMetadata { clip: usize, channel: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CanonicalValidationIssue {
    SceneSpace(SceneSpaceValidationError),
    ProvenanceArena(ProvenanceValidationIssue),
    MissingRoot,
    RootOutOfRange,
    RootReferenced,
    ChildOutOfRange {
        node: usize,
        child: u32,
    },
    NodeParentCount {
        node: usize,
        actual: u32,
    },
    HierarchyCycle {
        node: usize,
    },
    UnreachableNode {
        node: usize,
    },
    ReferenceOutOfRange {
        owner: CanonicalOwner,
        target: SceneEntityKind,
        target_index: u32,
    },
    ForeignProvenance {
        owner: CanonicalOwner,
    },
    ExtensionSchemaMissing {
        owner: CanonicalOwner,
        behavior: ExtensionBehaviorKind,
        schema_id: ExtensionSchemaId,
    },
    ExtensionSchemaOwnerMismatch {
        owner: CanonicalOwner,
        behavior: ExtensionBehaviorKind,
        expected: ExtensionSchemaOwner,
        actual: ExtensionSchemaOwner,
    },
    ExtensionSchemaBehaviorMismatch {
        owner: CanonicalOwner,
        expected: ExtensionSchemaBehavior,
        actual: ExtensionSchemaBehavior,
    },
    SourceLocatorValueBytesExceeded {
        provenance_node: usize,
        actual: usize,
        maximum: usize,
    },
    PackedAttributeSchemaMismatch {
        owner: CanonicalOwner,
        reason: RegisteredPackedEncodingError,
    },
    MetadataKeyBytesExceeded {
        owner: CanonicalExtensionOwner,
        entry: usize,
        actual: usize,
        maximum: usize,
    },
    MetadataStringBytesExceeded {
        owner: CanonicalExtensionOwner,
        entry: usize,
        actual: usize,
        maximum: usize,
    },
    DerivedGenerationMismatch {
        owner: CanonicalOwner,
    },
    DerivedInputStampMismatch {
        owner: CanonicalOwner,
    },
    MatrixAuthoredNodeTrsChannel {
        owner: CanonicalOwner,
        node: u32,
    },
    MorphWeightWidthMismatch {
        owner: CanonicalOwner,
        expected: usize,
        actual: usize,
    },
    SkinPaletteJointOutOfRange {
        instance: usize,
        logical_mesh: usize,
        primitive: usize,
        influence: usize,
        palette_joint: u32,
        palette_len: usize,
    },
    LimitExceeded {
        limit: CanonicalValidationLimit,
        actual: u64,
        maximum: u64,
    },
}

#[derive(Debug, Error)]
pub(crate) enum CanonicalValidationError {
    #[error("canonical scene validation failed with semantic issues")]
    Invalid {
        issues: Vec<CanonicalValidationIssue>,
        omitted_issue_count: u64,
        retained_issue_bytes: u64,
    },
    #[error("canonical scene validation scratch allocation failed")]
    ScratchAllocationFailed,
    #[error("canonical scene validation issue allocation failed")]
    IssueAllocationFailed,
    #[error("canonical scene accounting overflowed")]
    AccountingOverflow,
    #[error("canonical scene validation work {actual} exceeds limit {maximum}")]
    WorkLimitExceeded { actual: u64, maximum: u64 },
    #[error(transparent)]
    DerivedInputStamp(#[from] DerivedInputStampError),
    #[error(transparent)]
    MetadataFootprint(#[from] SceneMetadataError),
}

impl CanonicalValidationError {
    pub(crate) fn issues(&self) -> &[CanonicalValidationIssue] {
        match self {
            Self::Invalid { issues, .. } => issues,
            Self::ScratchAllocationFailed
            | Self::IssueAllocationFailed
            | Self::AccountingOverflow
            | Self::WorkLimitExceeded { .. }
            | Self::DerivedInputStamp(_)
            | Self::MetadataFootprint(_) => &[],
        }
    }

    pub(crate) const fn omitted_issue_count(&self) -> u64 {
        match self {
            Self::Invalid {
                omitted_issue_count,
                ..
            } => *omitted_issue_count,
            _ => 0,
        }
    }

    pub(crate) const fn retained_issue_bytes(&self) -> u64 {
        match self {
            Self::Invalid {
                retained_issue_bytes,
                ..
            } => *retained_issue_bytes,
            _ => 0,
        }
    }
}

struct IssueCollector {
    issues: Vec<CanonicalValidationIssue>,
    observed_issue_count: u64,
    retained_issue_bytes: u64,
    maximum_issues: u64,
    maximum_bytes: u64,
}

impl IssueCollector {
    const fn new(limits: SceneValidationLimits) -> Self {
        Self {
            issues: Vec::new(),
            observed_issue_count: 0,
            retained_issue_bytes: 0,
            maximum_issues: limits.max_diagnostic_issues(),
            maximum_bytes: limits.max_diagnostic_bytes(),
        }
    }

    fn record(&mut self, issue: CanonicalValidationIssue) -> Result<(), CanonicalValidationError> {
        self.observed_issue_count = self
            .observed_issue_count
            .checked_add(1)
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
        let stored = u64::try_from(self.issues.len())
            .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
        if stored >= self.maximum_issues {
            return Ok(());
        }

        if self.issues.len() == self.issues.capacity() {
            let desired = self
                .issues
                .len()
                .checked_add(1)
                .ok_or(CanonicalValidationError::AccountingOverflow)?;
            let growth = GrowableCapacityPlan::for_type::<CanonicalValidationIssue>(
                self.issues.len(),
                self.issues.capacity(),
                desired,
            )
            .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
            let plan = growth;
            let envelope_bytes = issue_capacity_bytes(plan.envelope_capacity())?;
            if envelope_bytes > self.maximum_bytes {
                return Ok(());
            }
            self.issues
                .try_reserve_exact(plan.allocator_additional())
                .map_err(|_| CanonicalValidationError::IssueAllocationFailed)?;
            plan.finish(self.issues.len(), self.issues.capacity())
                .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
            self.retained_issue_bytes = issue_capacity_bytes(self.issues.capacity())?;
        }
        self.issues.push(issue);
        Ok(())
    }

    const fn is_empty(&self) -> bool {
        self.observed_issue_count == 0
    }

    fn into_parts(
        self,
    ) -> Result<(Vec<CanonicalValidationIssue>, u64, u64), CanonicalValidationError> {
        let stored = u64::try_from(self.issues.len())
            .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
        let omitted = self
            .observed_issue_count
            .checked_sub(stored)
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
        Ok((self.issues, omitted, self.retained_issue_bytes))
    }
}

#[derive(Debug, Error)]
pub(crate) enum CanonicalValidationFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Validation(#[from] CanonicalValidationError),
    #[error("canonical scene validation control failed")]
    Control(#[source] E),
}

#[must_use = "dropping validation success releases its canonical reservation"]
pub(crate) struct CanonicalValidationSuccess<R> {
    scene: CanonicalValidatedScene,
    canonical_output: R,
}

impl<R> CanonicalValidationSuccess<R> {
    pub(crate) const fn stats(&self) -> SceneValidationStats {
        self.scene.validation_stats()
    }

    pub(crate) fn try_publish<E>(
        self,
        publish_reservation: impl FnOnce(&mut R) -> Result<(), E>,
    ) -> Result<CanonicalValidatedScene, E> {
        let Self {
            scene,
            canonical_output,
        } = self;
        try_publish_coupled(scene, canonical_output, publish_reservation)
    }

    pub(crate) fn try_publish_with_companion<C, E, F>(
        self,
        companion: C,
        publish_reservation: impl FnOnce(&mut R, &mut C) -> Result<(), E>,
        finalize_failure: impl FnOnce(E, C) -> F,
    ) -> Result<(CanonicalValidatedScene, C), F> {
        let Self {
            scene,
            canonical_output,
        } = self;
        try_publish_coupled_with(
            scene,
            companion,
            canonical_output,
            publish_reservation,
            finalize_failure,
        )
    }
}

impl CanonicalValidationSuccess<()> {
    fn into_unreserved_scene(self) -> CanonicalValidatedScene {
        self.scene
    }
}

type ControlledValidationResult<T, C> =
    Result<T, CanonicalValidationFailure<<C as SceneValidationControl>::Error>>;

struct ValidationEvidence<'a, C: SceneValidationControl> {
    control: &'a mut C,
    limits: SceneValidationLimits,
    counts: SceneCanonicalCounts,
    work: u64,
    maximum_depth: u64,
    canonical_bytes: u64,
    scratch_peak_bytes: u64,
    work_since_poll: u64,
}

impl<'a, C: SceneValidationControl> ValidationEvidence<'a, C> {
    fn with_initial_work(
        limits: SceneValidationLimits,
        control: &'a mut C,
        initial_work: u64,
    ) -> Self {
        Self {
            control,
            limits,
            counts: SceneCanonicalCounts::default(),
            work: initial_work,
            maximum_depth: 0,
            canonical_bytes: 0,
            scratch_peak_bytes: 0,
            work_since_poll: 0,
        }
    }

    fn add_work(&mut self, amount: u64) -> ControlledValidationResult<(), C> {
        self.add_typed_work(SceneValidationWork::Traversal, amount)
    }

    fn add_typed_work(
        &mut self,
        kind: SceneValidationWork,
        amount: u64,
    ) -> ControlledValidationResult<(), C> {
        let mut remaining = amount;
        while remaining > 0 {
            let until_poll = SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll;
            let chunk = remaining.min(until_poll);
            let next_work = self
                .work
                .checked_add(chunk)
                .ok_or(CanonicalValidationError::AccountingOverflow)?;
            if next_work > self.limits.max_work() {
                return Err(CanonicalValidationError::WorkLimitExceeded {
                    actual: next_work,
                    maximum: self.limits.max_work(),
                }
                .into());
            }
            self.control
                .charge_work(kind, chunk)
                .map_err(CanonicalValidationFailure::Control)?;
            self.work = next_work;
            self.work_since_poll += chunk;
            remaining -= chunk;
            if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
                self.poll_now()?;
            }
        }
        Ok(())
    }

    const fn work_until_poll(&self) -> u64 {
        SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll
    }

    fn poll_now(&mut self) -> ControlledValidationResult<(), C> {
        self.control
            .poll()
            .map_err(CanonicalValidationFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn add_reference(&mut self) -> ControlledValidationResult<(), C> {
        self.counts.references = self
            .counts
            .references
            .checked_add(1)
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
        self.add_typed_work(SceneValidationWork::Reference, 1)
    }

    fn add_edge(&mut self) -> ControlledValidationResult<(), C> {
        self.counts.edges = self
            .counts
            .edges
            .checked_add(1)
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
        self.add_typed_work(SceneValidationWork::GraphEdge, 1)
    }

    fn add_entities(&mut self, amount: usize) -> ControlledValidationResult<(), C> {
        let amount = checked_len(amount)?;
        self.counts.entities = self
            .counts
            .entities
            .checked_add(amount)
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
        self.add_typed_work(SceneValidationWork::Entity, amount)
    }

    fn add_payload_cells(&mut self, amount: usize) -> ControlledValidationResult<(), C> {
        let amount = checked_len(amount)?;
        self.counts.payload_cells = self
            .counts
            .payload_cells
            .checked_add(amount)
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
        self.add_typed_work(SceneValidationWork::PayloadCell, amount)
    }

    fn add_metadata_entry(&mut self, bytes: usize) -> ControlledValidationResult<(), C> {
        let bytes = checked_len(bytes)?;
        self.counts.metadata_entries = self
            .counts
            .metadata_entries
            .checked_add(1)
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
        self.counts.metadata_bytes = self
            .counts
            .metadata_bytes
            .checked_add(bytes)
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
        self.add_typed_work(SceneValidationWork::MetadataEntry, 1)?;
        self.add_typed_work(SceneValidationWork::MetadataByte, bytes)
    }

    fn reserve_scratch(&mut self, bytes: u64) -> ControlledValidationResult<C::Reservation, C> {
        self.scratch_peak_bytes = self.scratch_peak_bytes.max(bytes);
        self.control
            .reserve(SceneReservationKind::ValidationScratch, bytes)
            .map_err(CanonicalValidationFailure::Control)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> ControlledValidationResult<(), C> {
        self.scratch_peak_bytes = self.scratch_peak_bytes.max(bytes);
        self.control
            .resize_reservation(reservation, SceneReservationKind::ValidationScratch, bytes)
            .map_err(CanonicalValidationFailure::Control)
    }

    fn stats(&self) -> SceneValidationStats {
        SceneValidationStats::new(
            self.counts,
            SceneFootprint::new(self.canonical_bytes),
            self.work,
            self.maximum_depth,
            self.scratch_peak_bytes,
        )
    }
}

pub(crate) fn validate_canonical_scene(
    draft: CanonicalSceneDraft,
    limits: SceneValidationLimits,
) -> Result<CanonicalValidatedScene, CanonicalValidationError> {
    let mut control = NoopSceneValidationControl;
    match validate_canonical_scene_with(draft, limits, &mut control) {
        Ok(success) => Ok(success.into_unreserved_scene()),
        Err(CanonicalValidationFailure::Validation(error)) => Err(error),
        Err(CanonicalValidationFailure::Control(error)) => match error {},
    }
}

pub(crate) fn validate_canonical_scene_with<C: SceneValidationControl>(
    draft: CanonicalSceneDraft,
    limits: SceneValidationLimits,
    control: &mut C,
) -> ControlledValidationResult<CanonicalValidationSuccess<C::Reservation>, C> {
    let data = draft.data;
    let certificate = validate_canonical_scene_data_with(data, limits, control)?;
    let canonical_output = control
        .reserve(
            SceneReservationKind::CanonicalOutput,
            certificate.stats().footprint().accounted_payload_bytes(),
        )
        .map_err(CanonicalValidationFailure::Control)?;
    let scene = certificate.into_scene();
    let success = CanonicalValidationSuccess {
        scene,
        canonical_output,
    };
    control
        .poll()
        .map_err(CanonicalValidationFailure::Control)?;
    Ok(success)
}

pub(crate) struct CanonicalValidationCertificate {
    data: CanonicalSceneData,
    stats: SceneValidationStats,
    proof: CanonicalValidationProof,
}

impl CanonicalValidationCertificate {
    pub(crate) const fn stats(&self) -> SceneValidationStats {
        self.stats
    }

    pub(crate) fn into_scene(self) -> CanonicalValidatedScene {
        CanonicalValidatedScene::from_validated_parts(self.data, self.stats, self.proof)
    }
}

pub(crate) fn validate_canonical_scene_data_with<C: SceneValidationControl>(
    data: CanonicalSceneData,
    limits: SceneValidationLimits,
    control: &mut C,
) -> ControlledValidationResult<CanonicalValidationCertificate, C> {
    validate_canonical_scene_data_with_initial_work(data, limits, 0, control)
}

pub(crate) fn validate_canonical_scene_data_with_initial_work<C: SceneValidationControl>(
    data: CanonicalSceneData,
    limits: SceneValidationLimits,
    initial_work: u64,
    control: &mut C,
) -> ControlledValidationResult<CanonicalValidationCertificate, C> {
    if initial_work > limits.max_work() {
        return Err(CanonicalValidationError::WorkLimitExceeded {
            actual: initial_work,
            maximum: limits.max_work(),
        }
        .into());
    }
    let mut issues = IssueCollector::new(limits);
    let mut evidence = ValidationEvidence::with_initial_work(limits, control, initial_work);
    evidence.poll_now()?;
    measure_top_level_counts(&data, &mut evidence)?;
    enforce_top_level_limits(&mut issues, &evidence, limits)?;
    if !issues.is_empty() {
        let (issues, omitted_issue_count, retained_issue_bytes) = issues.into_parts()?;
        return Err(CanonicalValidationError::Invalid {
            issues,
            omitted_issue_count,
            retained_issue_bytes,
        }
        .into());
    }
    if let Err(error) = data.scene_space.validate() {
        record_issue(&mut issues, CanonicalValidationIssue::SceneSpace(error))?;
    }
    measure_provenance_and_retained_footprint(&data, &mut issues, &mut evidence)?;
    validate_derived_bounds(&data, &mut issues, &mut evidence)?;
    validate_scene_payloads(&data, limits, &mut issues, &mut evidence)?;
    validate_hierarchy(&data, &mut issues, &mut evidence)?;
    validate_entity_references(&data, &mut issues, &mut evidence)?;
    enforce_limits(&mut issues, &evidence, limits)?;

    if !issues.is_empty() {
        let (issues, omitted_issue_count, retained_issue_bytes) = issues.into_parts()?;
        return Err(CanonicalValidationError::Invalid {
            issues,
            omitted_issue_count,
            retained_issue_bytes,
        }
        .into());
    }

    let stats = evidence.stats();
    Ok(CanonicalValidationCertificate {
        data,
        stats,
        proof: CanonicalValidationProof { _private: () },
    })
}

fn measure_provenance_and_retained_footprint<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    let provenance_footprint_bytes =
        match validate_provenance_arena_controlled(data.provenance_arena.as_ref(), &mut || {
            evidence.add_work(1)
        }) {
            Ok(validated) => validated.stats().retained_footprint().total_bytes(),
            Err(error) => match error {
                ControlledProvenanceValidationError::Validation(error) => {
                    record_issue(
                        issues,
                        CanonicalValidationIssue::ProvenanceArena(error.issue()),
                    )?;
                    0
                }
                ControlledProvenanceValidationError::Control(error) => return Err(error),
            },
        };
    measure_retained_footprint(data, provenance_footprint_bytes, evidence)
}

fn validate_derived_bounds<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    for (logical_mesh_index, logical_mesh) in data.logical_meshes.iter().enumerate() {
        for (primitive_index, primitive) in logical_mesh.primitives().iter().enumerate() {
            validate_derived_value(
                primitive.bounds().derived(),
                CanonicalOwner::Primitive {
                    logical_mesh: logical_mesh_index,
                    primitive: primitive_index,
                },
                data.generation,
                |evidence| {
                    let mut charge_work = |amount| evidence.add_work(amount);
                    unwrap_controlled_stamp::<_, C>(primitive_bounds_input_stamp_controlled(
                        primitive,
                        logical_mesh.default_morph_weights(),
                        &mut charge_work,
                    ))
                },
                issues,
                evidence,
            )?;
        }
        validate_derived_value(
            logical_mesh.bounds().derived(),
            CanonicalOwner::Entity {
                kind: SceneEntityKind::LogicalMesh,
                index: logical_mesh_index,
            },
            data.generation,
            |evidence| {
                let mut charge_work = |amount| evidence.add_work(amount);
                unwrap_controlled_stamp::<_, C>(logical_mesh_bounds_input_stamp_controlled(
                    logical_mesh,
                    &mut charge_work,
                ))
            },
            issues,
            evidence,
        )?;
    }
    validate_derived_value(
        data.scene_bounds.derived(),
        CanonicalOwner::Scene,
        data.generation,
        |evidence| {
            let mut charge_work = |amount| evidence.add_work(amount);
            unwrap_controlled_stamp::<_, C>(scene_bounds_input_stamp_controlled(
                data,
                &mut charge_work,
            ))
        },
        issues,
        evidence,
    )
}

fn validate_derived_value<T, F, C>(
    derived: &Derived<T>,
    owner: CanonicalOwner,
    expected_generation: SceneGenerationId,
    expected_stamp: F,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C>
where
    C: SceneValidationControl,
    F: FnOnce(
        &mut ValidationEvidence<'_, C>,
    ) -> ControlledValidationResult<DerivedInputStampEvidence, C>,
{
    if !derived.is_valid() {
        return Ok(());
    }
    if derived.generation() != Some(expected_generation) {
        record_issue(
            issues,
            CanonicalValidationIssue::DerivedGenerationMismatch { owner },
        )?;
    }
    let expected_stamp = expected_stamp(evidence)?;
    if derived.input_stamp() != Some(&expected_stamp.stamp()) {
        record_issue(
            issues,
            CanonicalValidationIssue::DerivedInputStampMismatch { owner },
        )?;
    }
    Ok(())
}

fn unwrap_controlled_stamp<T, C: SceneValidationControl>(
    result: Result<T, ControlledDerivedInputStampError<CanonicalValidationFailure<C::Error>>>,
) -> ControlledValidationResult<T, C> {
    match result {
        Ok(value) => Ok(value),
        Err(ControlledDerivedInputStampError::Stamp(error)) => {
            Err(CanonicalValidationError::from(error).into())
        }
        Err(ControlledDerivedInputStampError::Control(error)) => Err(error),
    }
}

fn unwrap_controlled_geometry_footprint<C: SceneValidationControl>(
    result: Result<
        GeometryRetainedFootprint,
        ControlledGeometryStorageError<CanonicalValidationFailure<C::Error>>,
    >,
) -> ControlledValidationResult<usize, C> {
    match result {
        Ok(footprint) => Ok(footprint.bytes()),
        Err(ControlledGeometryStorageError::Storage(_)) => {
            Err(CanonicalValidationError::AccountingOverflow.into())
        }
        Err(ControlledGeometryStorageError::Control(error)) => Err(error),
    }
}

fn unwrap_controlled_retained_footprint<C: SceneValidationControl>(
    result: Result<usize, ControlledRetainedFootprintError<CanonicalValidationFailure<C::Error>>>,
) -> ControlledValidationResult<usize, C> {
    match result {
        Ok(bytes) => Ok(bytes),
        Err(ControlledRetainedFootprintError::AccountingOverflow) => {
            Err(CanonicalValidationError::AccountingOverflow.into())
        }
        Err(ControlledRetainedFootprintError::Control(error)) => Err(error),
    }
}

fn measure_top_level_counts<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    evidence.counts.nodes = checked_len(data.nodes.len())?;
    evidence.counts.entities = checked_sum_lengths(&[
        data.nodes.len(),
        data.mesh_instances.len(),
        data.logical_meshes.len(),
        data.materials.len(),
        data.images.len(),
        data.samplers.len(),
        data.textures.len(),
        data.skins.len(),
        data.animation_clips.len(),
        data.cameras.len(),
        data.lights.len(),
    ])?;
    Ok(())
}

fn measure_retained_footprint<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    provenance_footprint_bytes: usize,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    let mut canonical_bytes = owned_collection_bytes(&data.nodes, evidence, |node, evidence| {
        let mut charge_work = || evidence.add_work(1);
        unwrap_controlled_retained_footprint::<C>(
            node.retained_allocation_bytes_controlled(&mut charge_work),
        )
    })?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_collection_bytes(&data.mesh_instances, evidence, |instance, evidence| {
            let mut charge_work = || evidence.add_work(1);
            unwrap_controlled_geometry_footprint::<C>(
                instance.retained_footprint_controlled(&mut charge_work),
            )
        })?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        owned_collection_bytes(&data.logical_meshes, evidence, |mesh, evidence| {
            let mut charge_work = || evidence.add_work(1);
            unwrap_controlled_geometry_footprint::<C>(
                mesh.retained_footprint_controlled(&mut charge_work),
            )
        })?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_collection_bytes(&data.materials, evidence, |material, evidence| {
            let mut charge_work = || evidence.add_work(1);
            unwrap_controlled_retained_footprint::<C>(
                material.retained_allocation_bytes_controlled(&mut charge_work),
            )
        })?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_collection_bytes(&data.images, evidence, |image, _| {
            Ok(image.retained_allocation_bytes())
        })?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_collection_bytes(&data.samplers, evidence, |_, _| Ok(0))?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_collection_bytes(&data.textures, evidence, |_, _| Ok(0))?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_collection_bytes(&data.skins, evidence, |skin, _| {
            Ok(skin.retained_allocation_bytes())
        })?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_collection_bytes(&data.animation_clips, evidence, |clip, _| {
            Ok(clip.retained_allocation_bytes())
        })?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_collection_bytes(&data.cameras, evidence, |camera, evidence| {
            let mut charge_work = || evidence.add_work(1);
            unwrap_controlled_retained_footprint::<C>(
                camera.retained_allocation_bytes_controlled(&mut charge_work),
            )
        })?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_collection_bytes(&data.lights, evidence, |light, evidence| {
            let mut charge_work = || evidence.add_work(1);
            unwrap_controlled_retained_footprint::<C>(
                light.retained_allocation_bytes_controlled(&mut charge_work),
            )
        })?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_value_bytes(&data.extension_schemas, evidence)?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_value_bytes(&data.provenance, evidence)?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        shared_value_bytes(&data.metadata, evidence)?,
    )?;
    canonical_bytes = checked_add_bytes(canonical_bytes, provenance_footprint_bytes)?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        data.provenance_arena
            .checked_allocation_footprint()
            .map_err(|_| CanonicalValidationError::AccountingOverflow)?,
    )?;
    canonical_bytes = checked_add_bytes(
        canonical_bytes,
        data.scene_bounds
            .retained_producer_allocation_bytes()
            .map_err(|_| CanonicalValidationError::AccountingOverflow)?,
    )?;
    evidence.canonical_bytes =
        u64::try_from(canonical_bytes).map_err(|_| CanonicalValidationError::AccountingOverflow)?;
    Ok(())
}

fn shared_collection_bytes<T, C: SceneValidationControl>(
    values: &Vec<SharedStorage<T>>,
    evidence: &mut ValidationEvidence<'_, C>,
    mut nested_bytes: impl FnMut(
        &T,
        &mut ValidationEvidence<'_, C>,
    ) -> ControlledValidationResult<usize, C>,
) -> ControlledValidationResult<usize, C> {
    let mut bytes = outer_vec_bytes(values)?;
    for value in values {
        evidence.add_work(1)?;
        bytes = checked_add_bytes(
            bytes,
            value
                .checked_allocation_footprint()
                .map_err(|_| CanonicalValidationError::AccountingOverflow)?,
        )?;
        bytes = checked_add_bytes(bytes, nested_bytes(value.as_ref(), evidence)?)?;
    }
    Ok(bytes)
}

fn owned_collection_bytes<T, C: SceneValidationControl>(
    values: &Vec<T>,
    evidence: &mut ValidationEvidence<'_, C>,
    mut nested_bytes: impl FnMut(
        &T,
        &mut ValidationEvidence<'_, C>,
    ) -> ControlledValidationResult<usize, C>,
) -> ControlledValidationResult<usize, C> {
    let mut bytes = outer_vec_bytes(values)?;
    for value in values {
        evidence.add_work(1)?;
        bytes = checked_add_bytes(bytes, nested_bytes(value, evidence)?)?;
    }
    Ok(bytes)
}

fn shared_value_bytes<T, C: SceneValidationControl>(
    value: &SharedStorage<T>,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<usize, C> {
    evidence.add_work(1)?;
    value
        .checked_allocation_footprint()
        .map_err(|_| CanonicalValidationError::AccountingOverflow.into())
}

fn validate_scene_payloads<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    limits: SceneValidationLimits,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    validate_provenance_attachments(
        data,
        CanonicalOwner::Scene,
        data.provenance.as_ref(),
        issues,
        evidence,
    )?;
    validate_provenance_schema_closure(data, issues, evidence)?;
    if let Some(schema_id) = data.scene_space.tangent_basis.schema_id() {
        let _ = validate_schema_use(
            data,
            CanonicalOwner::Scene,
            ExtensionBehaviorKind::SceneSpaceTangentBasis,
            schema_id,
            issues,
            evidence,
        )?;
    }
    if let Some(schema_id) = data.scene_space.tangent_handedness_encoding.schema_id() {
        let _ = validate_schema_use(
            data,
            CanonicalOwner::Scene,
            ExtensionBehaviorKind::SceneSpaceTangentHandednessEncoding,
            schema_id,
            issues,
            evidence,
        )?;
    }
    validate_extension_provenance(
        data,
        CanonicalExtensionOwner::SceneMetadata,
        data.metadata.as_ref().entries(),
        limits,
        issues,
        evidence,
    )?;

    for (node_index, node) in data.nodes.iter().enumerate() {
        let owner = CanonicalOwner::Entity {
            kind: SceneEntityKind::Node,
            index: node_index,
        };
        validate_provenance_attachments(data, owner, node.provenance(), issues, evidence)?;
        validate_extension_provenance(
            data,
            CanonicalExtensionOwner::Entity {
                kind: SceneEntityKind::Node,
                index: node_index,
            },
            node.extensions(),
            limits,
            issues,
            evidence,
        )?;
    }

    for (instance_index, instance) in data.mesh_instances.iter().enumerate() {
        let instance = instance.as_ref();
        if let Some(weights) = instance.morph_weight_override() {
            evidence.add_payload_cells(weights.len())?;
        }
        validate_provenance_attachments(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::MeshInstance,
                index: instance_index,
            },
            instance.provenance(),
            issues,
            evidence,
        )?;
        for (attribute_index, attribute) in instance.attributes().iter().enumerate() {
            evidence.add_entities(1)?;
            measure_attribute_payload(attribute.payload(), evidence)?;
            let owner = CanonicalOwner::Attribute {
                owner: CanonicalAttributeOwner::MeshInstance {
                    instance: instance_index,
                },
                attribute: attribute_index,
            };
            validate_provenance(data, owner, *attribute.provenance(), issues, evidence)?;
            validate_attribute_schema(data, owner, attribute, issues, evidence)?;
        }
    }

    for (logical_mesh_index, logical_mesh) in data.logical_meshes.iter().enumerate() {
        evidence.add_payload_cells(logical_mesh.default_morph_weights().len())?;
        evidence.add_entities(
            logical_mesh
                .morph_target_definitions()
                .len()
                .checked_add(logical_mesh.primitives().len())
                .ok_or(CanonicalValidationError::AccountingOverflow)?,
        )?;
        validate_provenance_attachments(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::LogicalMesh,
                index: logical_mesh_index,
            },
            logical_mesh.provenance(),
            issues,
            evidence,
        )?;
        for (target_index, target) in logical_mesh.morph_target_definitions().iter().enumerate() {
            validate_provenance(
                data,
                CanonicalOwner::MorphTargetDefinition {
                    logical_mesh: logical_mesh_index,
                    target: target_index,
                },
                target.provenance(),
                issues,
                evidence,
            )?;
        }
        for (primitive_index, primitive) in logical_mesh.primitives().iter().enumerate() {
            if let Some(boundaries) = primitive.topology().polygon_boundaries() {
                evidence.add_payload_cells(boundaries.face_corner_counts().len())?;
            }
            if let Some(indices) = primitive.index_stream().indices() {
                evidence.add_payload_cells(indices.len())?;
            }
            if let Some(influences) = primitive.influences() {
                evidence.add_payload_cells(
                    influences
                        .vertex_offsets()
                        .len()
                        .checked_add(influences.influences().len())
                        .ok_or(CanonicalValidationError::AccountingOverflow)?,
                )?;
            }
            let primitive_owner = CanonicalOwner::Primitive {
                logical_mesh: logical_mesh_index,
                primitive: primitive_index,
            };
            validate_provenance_attachments(
                data,
                primitive_owner,
                primitive.provenance(),
                issues,
                evidence,
            )?;
            for (attribute_index, attribute) in primitive.attributes().iter().enumerate() {
                evidence.add_entities(1)?;
                measure_attribute_payload(attribute.payload(), evidence)?;
                let owner = CanonicalOwner::Attribute {
                    owner: CanonicalAttributeOwner::Primitive {
                        logical_mesh: logical_mesh_index,
                        primitive: primitive_index,
                    },
                    attribute: attribute_index,
                };
                validate_provenance(data, owner, *attribute.provenance(), issues, evidence)?;
                validate_attribute_schema(data, owner, attribute, issues, evidence)?;
            }
            for (target_index, target) in primitive.morph_targets().iter().enumerate() {
                evidence.add_entities(1)?;
                for semantic in [
                    MorphDeltaSemantic::Position,
                    MorphDeltaSemantic::Normal,
                    MorphDeltaSemantic::Tangent,
                ] {
                    if let Some(delta) = target.delta(semantic) {
                        evidence.add_entities(1)?;
                        measure_attribute_payload(delta.payload(), evidence)?;
                        validate_provenance(
                            data,
                            CanonicalOwner::MorphDelta {
                                logical_mesh: logical_mesh_index,
                                primitive: primitive_index,
                                target: target_index,
                                semantic,
                            },
                            delta.provenance(),
                            issues,
                            evidence,
                        )?;
                    }
                }
            }
        }
    }

    for (material_index, material) in data.materials.iter().enumerate() {
        let material = material.as_ref();
        validate_provenance(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::Material,
                index: material_index,
            },
            material.provenance(),
            issues,
            evidence,
        )?;
        validate_extension_provenance(
            data,
            CanonicalExtensionOwner::Entity {
                kind: SceneEntityKind::Material,
                index: material_index,
            },
            material.extensions(),
            limits,
            issues,
            evidence,
        )?;
        for schema_use in material.behavior_schema_uses() {
            let _ = validate_schema_use(
                data,
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::Material,
                    index: material_index,
                },
                appearance_extension_behavior(schema_use.behavior()),
                schema_use.schema_id(),
                issues,
                evidence,
            )?;
        }
        for (binding_index, binding) in material.texture_bindings().iter().enumerate() {
            evidence.add_entities(1)?;
            validate_provenance(
                data,
                CanonicalOwner::TextureBinding {
                    material: material_index,
                    binding: binding_index,
                },
                binding.provenance(),
                issues,
                evidence,
            )?;
            validate_extension_provenance(
                data,
                CanonicalExtensionOwner::TextureBinding {
                    material: material_index,
                    binding: binding_index,
                },
                binding.extensions(),
                limits,
                issues,
                evidence,
            )?;
            for schema_use in binding.behavior_schema_uses() {
                let _ = validate_schema_use(
                    data,
                    CanonicalOwner::TextureBinding {
                        material: material_index,
                        binding: binding_index,
                    },
                    appearance_extension_behavior(schema_use.behavior()),
                    schema_use.schema_id(),
                    issues,
                    evidence,
                )?;
            }
        }
    }

    for (image_index, image) in data.images.iter().enumerate() {
        match image.as_ref().payload().view() {
            ImagePayloadView::Encoded(payload) => {
                let embedded_bytes = checked_len(payload.bytes().len())?;
                evidence.counts.embedded_bytes = evidence
                    .counts
                    .embedded_bytes
                    .checked_add(embedded_bytes)
                    .ok_or(CanonicalValidationError::AccountingOverflow)?;
                evidence.add_payload_cells(payload.bytes().len())?;
            }
        }
        validate_provenance(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::Image,
                index: image_index,
            },
            image.as_ref().provenance(),
            issues,
            evidence,
        )?;
    }
    for (sampler_index, sampler) in data.samplers.iter().enumerate() {
        validate_provenance(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::Sampler,
                index: sampler_index,
            },
            sampler.as_ref().provenance(),
            issues,
            evidence,
        )?;
    }
    for (texture_index, texture) in data.textures.iter().enumerate() {
        validate_provenance(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::Texture,
                index: texture_index,
            },
            texture.as_ref().provenance(),
            issues,
            evidence,
        )?;
    }
    for (skin_index, skin) in data.skins.iter().enumerate() {
        evidence.add_payload_cells(skin.as_ref().inverse_bind_matrices().len())?;
        validate_provenance(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::Skin,
                index: skin_index,
            },
            skin.as_ref().provenance(),
            issues,
            evidence,
        )?;
    }
    for (clip_index, clip) in data.animation_clips.iter().enumerate() {
        let clip = clip.as_ref();
        validate_provenance(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::AnimationClip,
                index: clip_index,
            },
            clip.provenance(),
            issues,
            evidence,
        )?;
        if let Some(schema_id) = clip.time_domain().schema_id() {
            let _ = validate_schema_use(
                data,
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::AnimationClip,
                    index: clip_index,
                },
                ExtensionBehaviorKind::AnimationTimeDomain,
                schema_id,
                issues,
                evidence,
            )?;
        }
        for (channel_index, channel) in clip.channels().iter().enumerate() {
            evidence.add_entities(1)?;
            evidence.add_payload_cells(channel.authored_times().len())?;
            evidence.add_payload_cells(animation_value_cells(channel.values().view())?)?;
            let channel_owner = CanonicalOwner::AnimationChannel {
                clip: clip_index,
                channel: channel_index,
            };
            validate_provenance(data, channel_owner, channel.provenance(), issues, evidence)?;
            for schema_id in [
                channel.pre_extrapolation().schema_id(),
                channel.post_extrapolation().schema_id(),
            ]
            .into_iter()
            .flatten()
            {
                let _ = validate_schema_use(
                    data,
                    channel_owner,
                    ExtensionBehaviorKind::AnimationExtrapolation,
                    schema_id,
                    issues,
                    evidence,
                )?;
            }
            validate_extension_provenance(
                data,
                CanonicalExtensionOwner::AnimationTimingMetadata {
                    clip: clip_index,
                    channel: channel_index,
                },
                channel.timing_metadata().entries(),
                limits,
                issues,
                evidence,
            )?;
        }
    }

    for (camera_index, camera) in data.cameras.iter().enumerate() {
        let camera = camera.as_ref();
        validate_provenance(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::Camera,
                index: camera_index,
            },
            camera.provenance(),
            issues,
            evidence,
        )?;
        validate_extension_provenance(
            data,
            CanonicalExtensionOwner::Entity {
                kind: SceneEntityKind::Camera,
                index: camera_index,
            },
            camera.extensions(),
            limits,
            issues,
            evidence,
        )?;
    }
    for (light_index, light) in data.lights.iter().enumerate() {
        let light = light.as_ref();
        validate_provenance(
            data,
            CanonicalOwner::Entity {
                kind: SceneEntityKind::Light,
                index: light_index,
            },
            light.provenance(),
            issues,
            evidence,
        )?;
        validate_extension_provenance(
            data,
            CanonicalExtensionOwner::Entity {
                kind: SceneEntityKind::Light,
                index: light_index,
            },
            light.extensions(),
            limits,
            issues,
            evidence,
        )?;
    }
    Ok(())
}

fn appearance_extension_behavior(behavior: AppearanceBehaviorKind) -> ExtensionBehaviorKind {
    match behavior {
        AppearanceBehaviorKind::ShadingModel => ExtensionBehaviorKind::MaterialShadingModel,
        AppearanceBehaviorKind::AlphaMode => ExtensionBehaviorKind::MaterialAlphaMode,
        AppearanceBehaviorKind::TextureRole => ExtensionBehaviorKind::TextureRole,
        AppearanceBehaviorKind::CoordinateSource => ExtensionBehaviorKind::TextureCoordinateSource,
        AppearanceBehaviorKind::ChannelMapping => ExtensionBehaviorKind::TextureChannelMapping,
        AppearanceBehaviorKind::ColorInterpretation => {
            ExtensionBehaviorKind::TextureColorInterpretation
        }
        AppearanceBehaviorKind::BlendOperation => ExtensionBehaviorKind::TextureBlendOperation,
    }
}

fn measure_attribute_payload<C: SceneValidationControl>(
    payload: &crate::attribute::AttributePayload,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    let mut cells = usize::try_from(payload.explicit_values().element_count())
        .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
    if let Some(indices) = payload.sparse_indices() {
        cells = cells
            .checked_add(indices.len())
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
    }
    if let Some(default) = payload.default_value() {
        cells = cells
            .checked_add(
                usize::try_from(default.element_count())
                    .map_err(|_| CanonicalValidationError::AccountingOverflow)?,
            )
            .ok_or(CanonicalValidationError::AccountingOverflow)?;
    }
    evidence.add_payload_cells(cells)
}

fn animation_value_cells(
    values: AnimationValuesView<'_>,
) -> Result<usize, CanonicalValidationError> {
    match values {
        AnimationValuesView::Translations(samples) | AnimationValuesView::Scales(samples) => {
            fixed_animation_sample_cells(samples, 3)
        }
        AnimationValuesView::Rotations(samples) => fixed_animation_sample_cells(samples, 4),
        AnimationValuesView::MorphWeights { samples, .. } => morph_animation_sample_cells(samples),
    }
}

fn fixed_animation_sample_cells<Value, Tangent>(
    samples: AnimationSamplesView<'_, Value, Tangent>,
    cells_per_sample: usize,
) -> Result<usize, CanonicalValidationError> {
    let sample_count = match samples {
        AnimationSamplesView::Values(values) => values.len(),
        AnimationSamplesView::CubicSpline {
            in_tangents,
            values,
            out_tangents,
        } => in_tangents
            .len()
            .checked_add(values.len())
            .and_then(|count| count.checked_add(out_tangents.len()))
            .ok_or(CanonicalValidationError::AccountingOverflow)?,
    };
    sample_count
        .checked_mul(cells_per_sample)
        .ok_or(CanonicalValidationError::AccountingOverflow)
}

fn morph_animation_sample_cells(
    samples: AnimationSamplesView<'_, Vec<f32>, Vec<f32>>,
) -> Result<usize, CanonicalValidationError> {
    match samples {
        AnimationSamplesView::Values(values) => checked_row_cells(values),
        AnimationSamplesView::CubicSpline {
            in_tangents,
            values,
            out_tangents,
        } => checked_row_cells(in_tangents)?
            .checked_add(checked_row_cells(values)?)
            .and_then(|cells| cells.checked_add(checked_row_cells(out_tangents).ok()?))
            .ok_or(CanonicalValidationError::AccountingOverflow),
    }
}

fn checked_row_cells(rows: &[Vec<f32>]) -> Result<usize, CanonicalValidationError> {
    rows.iter().try_fold(0_usize, |cells, row| {
        cells
            .checked_add(row.len())
            .ok_or(CanonicalValidationError::AccountingOverflow)
    })
}

fn validate_attribute_schema<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    owner: CanonicalOwner,
    attribute: &crate::attribute::Attribute,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    if let Normalization::Named(normalization) = attribute.format().normalization() {
        let behavior = match normalization.signedness() {
            IntegerSignedness::Signed => ExtensionBehaviorKind::AttributeSignedNormalization,
            IntegerSignedness::Unsigned => ExtensionBehaviorKind::AttributeUnsignedNormalization,
        };
        let _ = validate_schema_use(
            data,
            owner,
            behavior,
            normalization.schema_id(),
            issues,
            evidence,
        )?;
    }
    if let Some(packed) = attribute.format().packed_encoding() {
        evidence.add_reference()?;
        if let Err(reason) = packed.validate_against_snapshot(data.extension_schemas.as_ref()) {
            record_issue(
                issues,
                CanonicalValidationIssue::PackedAttributeSchemaMismatch { owner, reason },
            )?;
        }
    }
    Ok(())
}

fn validate_schema_use<'a, C: SceneValidationControl>(
    data: &'a CanonicalSceneData,
    owner: CanonicalOwner,
    behavior: ExtensionBehaviorKind,
    schema_id: &ExtensionSchemaId,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<Option<&'a ExtensionSchema>, C> {
    evidence.add_reference()?;
    let Some(schema) = data.extension_schemas.as_ref().get(schema_id) else {
        record_issue(
            issues,
            CanonicalValidationIssue::ExtensionSchemaMissing {
                owner,
                behavior,
                schema_id: schema_id.clone(),
            },
        )?;
        return Ok(None);
    };
    let expected_owner = behavior.expected_owner();
    if schema.owner() != expected_owner {
        record_issue(
            issues,
            CanonicalValidationIssue::ExtensionSchemaOwnerMismatch {
                owner,
                behavior,
                expected: expected_owner,
                actual: schema.owner(),
            },
        )?;
        return Ok(None);
    }
    let expected_behavior = ExtensionSchemaBehavior::Canonical(behavior);
    if schema.behavior() != expected_behavior {
        record_issue(
            issues,
            CanonicalValidationIssue::ExtensionSchemaBehaviorMismatch {
                owner,
                expected: expected_behavior,
                actual: schema.behavior(),
            },
        )?;
        return Ok(None);
    }
    Ok(Some(schema))
}

fn validate_provenance_attachments<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    owner: CanonicalOwner,
    attachments: &ProvenanceAttachments,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    for provenance in attachments.iter() {
        validate_provenance(data, owner, provenance, issues, evidence)?;
    }
    Ok(())
}

fn validate_provenance_schema_closure<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    for (provenance_node, node) in data.provenance_arena.as_ref().iter().enumerate() {
        evidence.add_work(1)?;
        let ProvenanceNodeView::Source(identity) = node.node() else {
            continue;
        };
        let Some(schema_id) = identity.locator().schema_id() else {
            continue;
        };
        let owner = CanonicalOwner::ProvenanceNode {
            index: provenance_node,
        };
        let Some(schema) = validate_schema_use(
            data,
            owner,
            ExtensionBehaviorKind::SourceObjectLocator,
            schema_id,
            issues,
            evidence,
        )?
        else {
            continue;
        };
        let ExtensionValueSchema::String { max_bytes } = schema.value() else {
            continue;
        };
        let maximum = usize::try_from(*max_bytes)
            .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
        let actual = identity
            .locator()
            .namespaced_value()
            .map_or(0, |value| value.as_str().len());
        if actual > maximum {
            record_issue(
                issues,
                CanonicalValidationIssue::SourceLocatorValueBytesExceeded {
                    provenance_node,
                    actual,
                    maximum,
                },
            )?;
        }
    }
    Ok(())
}

fn validate_extension_provenance<'a, C: SceneValidationControl>(
    data: &CanonicalSceneData,
    owner: CanonicalExtensionOwner,
    entries: impl IntoIterator<Item = &'a ExtensionEntry>,
    limits: SceneValidationLimits,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    for (entry_index, entry) in entries.into_iter().enumerate() {
        measure_extension_entry(owner, entry_index, entry, limits, issues, evidence)?;
        validate_provenance(
            data,
            CanonicalOwner::ExtensionEntry {
                owner,
                entry: entry_index,
            },
            entry.provenance(),
            issues,
            evidence,
        )?;
    }
    Ok(())
}

fn measure_extension_entry<C: SceneValidationControl>(
    owner: CanonicalExtensionOwner,
    entry_index: usize,
    entry: &ExtensionEntry,
    limits: SceneValidationLimits,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    evidence.add_payload_cells(entry.value().cell_count())?;
    if entry.classification() != ExtensionEntryClassification::DescriptiveMetadata {
        return Ok(());
    }

    let key_bytes = entry.key().as_str().len();
    let metadata_bytes = key_bytes
        .checked_add(entry.value().dynamic_payload_bytes())
        .ok_or(CanonicalValidationError::AccountingOverflow)?;
    evidence.add_metadata_entry(metadata_bytes)?;
    if key_bytes > limits.max_metadata_key_bytes() {
        record_issue(
            issues,
            CanonicalValidationIssue::MetadataKeyBytesExceeded {
                owner,
                entry: entry_index,
                actual: key_bytes,
                maximum: limits.max_metadata_key_bytes(),
            },
        )?;
    }
    if let ExtensionValueRef::String(value) = entry.value().as_ref()
        && value.len() > limits.max_metadata_string_bytes()
    {
        record_issue(
            issues,
            CanonicalValidationIssue::MetadataStringBytesExceeded {
                owner,
                entry: entry_index,
                actual: value.len(),
                maximum: limits.max_metadata_string_bytes(),
            },
        )?;
    }
    Ok(())
}

fn validate_provenance<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    owner: CanonicalOwner,
    provenance: Provenance,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    evidence.add_reference()?;
    if !data.provenance_arena.as_ref().contains(&provenance) {
        record_issue(
            issues,
            CanonicalValidationIssue::ForeignProvenance { owner },
        )?;
    }
    Ok(())
}

fn validate_hierarchy<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    let node_count = data.nodes.len();
    let Some(root) = data.root else {
        record_issue(issues, CanonicalValidationIssue::MissingRoot)?;
        return validate_node_payloads(data, issues, evidence);
    };
    let root = root
        .try_to_usize()
        .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
    if root >= node_count {
        record_issue(issues, CanonicalValidationIssue::RootOutOfRange)?;
        return validate_node_payloads(data, issues, evidence);
    }

    let indegree_plan = empty_scratch_capacity_plan::<u32>(node_count)?;
    let state_plan = empty_scratch_capacity_plan::<u8>(node_count)?;
    let stack_plan = empty_scratch_capacity_plan::<TraversalFrame>(node_count)?;
    let scratch_envelope = checked_add_u64(
        checked_add_u64(
            scratch_capacity_envelope(indegree_plan)?,
            scratch_capacity_envelope(state_plan)?,
        )?,
        scratch_capacity_envelope(stack_plan)?,
    )?;
    evidence.scratch_peak_bytes = evidence.scratch_peak_bytes.max(scratch_envelope);
    if scratch_envelope > evidence.limits.max_scratch_bytes() {
        return validate_node_payloads(data, issues, evidence);
    }
    let mut scratch = evidence.reserve_scratch(scratch_envelope)?;
    let (mut indegree, indegree_bytes): (Vec<u32>, u64) =
        apply_empty_scratch_capacity_plan(indegree_plan)?;
    let (mut state, state_bytes) = apply_empty_scratch_capacity_plan(state_plan)?;
    let (mut stack, stack_bytes) = apply_empty_scratch_capacity_plan(stack_plan)?;
    let actual_scratch =
        checked_add_u64(checked_add_u64(indegree_bytes, state_bytes)?, stack_bytes)?;
    evidence.resize_scratch(&mut scratch, actual_scratch)?;
    initialize_scratch_vec(&mut indegree, node_count, 0, evidence)?;
    initialize_scratch_vec(&mut state, node_count, 0, evidence)?;

    validate_node_payloads(data, issues, evidence)?;
    for (node_index, node) in data.nodes.iter().enumerate() {
        for child in node.children() {
            evidence.add_edge()?;
            evidence.add_reference()?;
            let child_u32 = child.as_u32();
            let child_index = child
                .try_to_usize()
                .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
            let Some(incoming) = indegree.get_mut(child_index) else {
                record_issue(
                    issues,
                    CanonicalValidationIssue::ChildOutOfRange {
                        node: node_index,
                        child: child_u32,
                    },
                )?;
                continue;
            };
            *incoming = incoming
                .checked_add(1)
                .ok_or(CanonicalValidationError::AccountingOverflow)?;
        }
    }

    if indegree[root] != 0 {
        record_issue(issues, CanonicalValidationIssue::RootReferenced)?;
    }
    for (node, incoming) in indegree.iter().copied().enumerate() {
        if node != root && incoming != 1 {
            record_issue(
                issues,
                CanonicalValidationIssue::NodeParentCount {
                    node,
                    actual: incoming,
                },
            )?;
        }
    }

    traverse_component(data, root, false, &mut state, &mut stack, issues, evidence)?;
    for component_root in 0..node_count {
        if state[component_root] == 0 {
            traverse_component(
                data,
                component_root,
                true,
                &mut state,
                &mut stack,
                issues,
                evidence,
            )?;
        }
    }
    Ok(())
}

fn traverse_component<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    component_root: usize,
    unreachable: bool,
    state: &mut [u8],
    stack: &mut Vec<TraversalFrame>,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    stack.clear();
    state[component_root] = 1;
    stack.push(TraversalFrame {
        node: component_root,
        next_child: 0,
        depth: 1,
    });
    evidence.maximum_depth = evidence.maximum_depth.max(1);
    if unreachable {
        record_issue(
            issues,
            CanonicalValidationIssue::UnreachableNode {
                node: component_root,
            },
        )?;
    }

    while let Some(frame) = stack.last_mut() {
        evidence.add_work(1)?;
        let child = data.nodes[frame.node].child_at(frame.next_child);
        let Some(child) = child else {
            state[frame.node] = 2;
            stack.pop();
            continue;
        };
        frame.next_child += 1;
        let child_index = child
            .try_to_usize()
            .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
        if child_index >= data.nodes.len() {
            continue;
        }
        match state[child_index] {
            0 => {
                let depth = frame
                    .depth
                    .checked_add(1)
                    .ok_or(CanonicalValidationError::AccountingOverflow)?;
                evidence.maximum_depth = evidence.maximum_depth.max(depth);
                state[child_index] = 1;
                if unreachable {
                    record_issue(
                        issues,
                        CanonicalValidationIssue::UnreachableNode { node: child_index },
                    )?;
                }
                stack.push(TraversalFrame {
                    node: child_index,
                    next_child: 0,
                    depth,
                });
            }
            1 => record_issue(
                issues,
                CanonicalValidationIssue::HierarchyCycle { node: child_index },
            )?,
            2 => {}
            _ => return Err(CanonicalValidationError::AccountingOverflow.into()),
        }
    }
    Ok(())
}

fn validate_node_payloads<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    for (node_index, node) in data.nodes.iter().enumerate() {
        evidence.add_work(1)?;
        if let Some(camera) = node.camera_index() {
            validate_reference(
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::Node,
                    index: node_index,
                },
                camera,
                data.cameras.len(),
                issues,
                evidence,
            )?;
        }
        if let Some(light) = node.light_index() {
            validate_reference(
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::Node,
                    index: node_index,
                },
                light,
                data.lights.len(),
                issues,
                evidence,
            )?;
        }
    }
    Ok(())
}

fn validate_entity_references<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    let palette_summaries = build_mesh_palette_summaries(data, evidence)?;
    let palette_summaries = palette_summaries
        .as_ref()
        .map(|summaries| summaries.values.as_slice());

    for (texture_index, texture) in data.textures.iter().enumerate() {
        let owner = CanonicalOwner::Entity {
            kind: SceneEntityKind::Texture,
            index: texture_index,
        };
        validate_reference(
            owner,
            texture.as_ref().image_index(),
            data.images.len(),
            issues,
            evidence,
        )?;
        validate_reference(
            owner,
            texture.as_ref().sampler_index(),
            data.samplers.len(),
            issues,
            evidence,
        )?;
    }

    for (instance_index, instance) in data.mesh_instances.iter().enumerate() {
        let owner = CanonicalOwner::Entity {
            kind: SceneEntityKind::MeshInstance,
            index: instance_index,
        };
        validate_reference(
            owner,
            instance.as_ref().owner_index(),
            data.nodes.len(),
            issues,
            evidence,
        )?;
        validate_reference(
            owner,
            instance.as_ref().logical_mesh_index(),
            data.logical_meshes.len(),
            issues,
            evidence,
        )?;
        if let Some(skin) = instance.as_ref().skin_index() {
            validate_reference(owner, skin, data.skins.len(), issues, evidence)?;
        }
        validate_instance_owner_contracts(
            data,
            instance_index,
            instance.as_ref(),
            palette_summaries,
            issues,
        )?;
    }

    for (skin_index, skin) in data.skins.iter().enumerate() {
        let owner = CanonicalOwner::Entity {
            kind: SceneEntityKind::Skin,
            index: skin_index,
        };
        for joint in skin.as_ref().joints() {
            validate_reference(owner, *joint, data.nodes.len(), issues, evidence)?;
        }
        if let Some(root) = skin.as_ref().skeleton_root_hint() {
            validate_reference(owner, root, data.nodes.len(), issues, evidence)?;
        }
    }

    for (logical_mesh_index, logical_mesh) in data.logical_meshes.iter().enumerate() {
        for (primitive_index, primitive) in logical_mesh.primitives().iter().enumerate() {
            if let Some(material) = primitive.material_index() {
                validate_reference(
                    CanonicalOwner::Primitive {
                        logical_mesh: logical_mesh_index,
                        primitive: primitive_index,
                    },
                    material,
                    data.materials.len(),
                    issues,
                    evidence,
                )?;
            }
        }
    }

    for (material_index, material) in data.materials.iter().enumerate() {
        for (binding_index, binding) in material.as_ref().texture_bindings().iter().enumerate() {
            validate_reference(
                CanonicalOwner::TextureBinding {
                    material: material_index,
                    binding: binding_index,
                },
                binding.texture_index(),
                data.textures.len(),
                issues,
                evidence,
            )?;
        }
    }

    for (clip_index, clip) in data.animation_clips.iter().enumerate() {
        for (channel_index, channel) in clip.as_ref().channels().iter().enumerate() {
            let owner = CanonicalOwner::AnimationChannel {
                clip: clip_index,
                channel: channel_index,
            };
            match channel.target() {
                AnimationTarget::NodeTranslation(node)
                | AnimationTarget::NodeRotation(node)
                | AnimationTarget::NodeScale(node) => {
                    validate_reference(owner, node, data.nodes.len(), issues, evidence)?;
                    if let Some(target_node) = node
                        .get(&data.nodes)
                        .map_err(|_| CanonicalValidationError::AccountingOverflow)?
                        && target_node.authored_transform().kind() == AuthoredTransformKind::Matrix
                    {
                        record_issue(
                            issues,
                            CanonicalValidationIssue::MatrixAuthoredNodeTrsChannel {
                                owner,
                                node: node.as_u32(),
                            },
                        )?;
                    }
                }
                AnimationTarget::MeshInstanceMorphWeights(instance) => {
                    validate_reference(
                        owner,
                        instance,
                        data.mesh_instances.len(),
                        issues,
                        evidence,
                    )?;
                    if let Some(instance) = instance
                        .get(&data.mesh_instances)
                        .map_err(|_| CanonicalValidationError::AccountingOverflow)?
                        && let Some(mesh) = instance
                            .as_ref()
                            .logical_mesh_index()
                            .get(&data.logical_meshes)
                            .map_err(|_| CanonicalValidationError::AccountingOverflow)?
                        && let AnimationValuesView::MorphWeights { width, .. } =
                            channel.values().view()
                    {
                        let expected = mesh.morph_target_definitions().len();
                        if width != expected {
                            record_issue(
                                issues,
                                CanonicalValidationIssue::MorphWeightWidthMismatch {
                                    owner,
                                    expected,
                                    actual: width,
                                },
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct MeshPaletteSummary {
    maximum: Option<PaletteJointLocation>,
}

#[derive(Clone, Copy)]
struct PaletteJointLocation {
    primitive: usize,
    influence: usize,
    palette_joint: u32,
}

struct ReservedMeshPaletteSummaries<R> {
    values: Vec<MeshPaletteSummary>,
    _reservation: R,
}

fn build_mesh_palette_summaries<C: SceneValidationControl>(
    data: &CanonicalSceneData,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<Option<ReservedMeshPaletteSummaries<C::Reservation>>, C> {
    if data.logical_meshes.is_empty() {
        return Ok(None);
    }
    let plan = empty_scratch_capacity_plan::<MeshPaletteSummary>(data.logical_meshes.len())?;
    let scratch_envelope = scratch_capacity_envelope(plan)?;
    evidence.scratch_peak_bytes = evidence.scratch_peak_bytes.max(scratch_envelope);
    if evidence.scratch_peak_bytes > evidence.limits.max_scratch_bytes() {
        return Ok(None);
    }
    let mut reservation = evidence.reserve_scratch(scratch_envelope)?;
    let (mut values, actual_scratch) = apply_empty_scratch_capacity_plan(plan)?;
    evidence.resize_scratch(&mut reservation, actual_scratch)?;
    initialize_scratch_vec(
        &mut values,
        data.logical_meshes.len(),
        MeshPaletteSummary::default(),
        evidence,
    )?;

    for (logical_mesh, summary) in data.logical_meshes.iter().zip(&mut values) {
        for (primitive_index, primitive) in logical_mesh.primitives().iter().enumerate() {
            let Some(influences) = primitive.influences() else {
                continue;
            };
            for (influence_index, influence) in influences.influences().iter().enumerate() {
                evidence.add_work(1)?;
                let candidate = PaletteJointLocation {
                    primitive: primitive_index,
                    influence: influence_index,
                    palette_joint: influence.palette_joint(),
                };
                if summary
                    .maximum
                    .is_none_or(|current| candidate.palette_joint > current.palette_joint)
                {
                    summary.maximum = Some(candidate);
                }
            }
        }
    }
    Ok(Some(ReservedMeshPaletteSummaries {
        values,
        _reservation: reservation,
    }))
}

fn validate_instance_owner_contracts(
    data: &CanonicalSceneData,
    instance_index: usize,
    instance: &crate::geometry::MeshInstance,
    palette_summaries: Option<&[MeshPaletteSummary]>,
    issues: &mut IssueCollector,
) -> Result<(), CanonicalValidationError> {
    let Some(mesh) = instance
        .logical_mesh_index()
        .get(&data.logical_meshes)
        .map_err(|_| CanonicalValidationError::AccountingOverflow)?
    else {
        return Ok(());
    };
    let logical_mesh_index = instance
        .logical_mesh_index()
        .try_to_usize()
        .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
    let expected_morph_width = mesh.morph_target_definitions().len();
    if let Some(weights) = instance.morph_weight_override()
        && weights.len() != expected_morph_width
    {
        record_issue(
            issues,
            CanonicalValidationIssue::MorphWeightWidthMismatch {
                owner: CanonicalOwner::Entity {
                    kind: SceneEntityKind::MeshInstance,
                    index: instance_index,
                },
                expected: expected_morph_width,
                actual: weights.len(),
            },
        )?;
    }

    let Some(skin_index) = instance.skin_index() else {
        return Ok(());
    };
    let Some(skin) = skin_index
        .get(&data.skins)
        .map_err(|_| CanonicalValidationError::AccountingOverflow)?
    else {
        return Ok(());
    };
    let palette_len = skin.as_ref().joints().len();
    let Some(summary) = palette_summaries
        .and_then(|summaries| summaries.get(logical_mesh_index))
        .and_then(|summary| summary.maximum)
    else {
        return Ok(());
    };
    if usize::try_from(summary.palette_joint)
        .map_err(|_| CanonicalValidationError::AccountingOverflow)?
        >= palette_len
    {
        record_issue(
            issues,
            CanonicalValidationIssue::SkinPaletteJointOutOfRange {
                instance: instance_index,
                logical_mesh: logical_mesh_index,
                primitive: summary.primitive,
                influence: summary.influence,
                palette_joint: summary.palette_joint,
                palette_len,
            },
        )?;
    }
    Ok(())
}

fn validate_reference<I: SceneStorageIndex, C: SceneValidationControl>(
    owner: CanonicalOwner,
    target: I,
    target_len: usize,
    issues: &mut IssueCollector,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C> {
    evidence.add_reference()?;
    if target
        .try_to_usize()
        .map_err(|_| CanonicalValidationError::AccountingOverflow)?
        >= target_len
    {
        record_issue(
            issues,
            CanonicalValidationIssue::ReferenceOutOfRange {
                owner,
                target: I::KIND,
                target_index: target.as_u32(),
            },
        )?;
    }
    Ok(())
}

fn enforce_top_level_limits<C: SceneValidationControl>(
    issues: &mut IssueCollector,
    evidence: &ValidationEvidence<'_, C>,
    limits: SceneValidationLimits,
) -> Result<(), CanonicalValidationError> {
    for (limit, actual, maximum) in [
        (
            CanonicalValidationLimit::Nodes,
            evidence.counts.nodes,
            limits.max_nodes(),
        ),
        (
            CanonicalValidationLimit::Entities,
            evidence.counts.entities,
            limits.max_entities(),
        ),
    ] {
        if actual > maximum {
            record_issue(
                issues,
                CanonicalValidationIssue::LimitExceeded {
                    limit,
                    actual,
                    maximum,
                },
            )?;
        }
    }
    Ok(())
}

fn enforce_limits<C: SceneValidationControl>(
    issues: &mut IssueCollector,
    evidence: &ValidationEvidence<'_, C>,
    limits: SceneValidationLimits,
) -> Result<(), CanonicalValidationError> {
    let checks = [
        (
            CanonicalValidationLimit::Nodes,
            evidence.counts.nodes,
            limits.max_nodes(),
        ),
        (
            CanonicalValidationLimit::Edges,
            evidence.counts.edges,
            limits.max_edges(),
        ),
        (
            CanonicalValidationLimit::Depth,
            evidence.maximum_depth,
            limits.max_depth(),
        ),
        (
            CanonicalValidationLimit::Entities,
            evidence.counts.entities,
            limits.max_entities(),
        ),
        (
            CanonicalValidationLimit::References,
            evidence.counts.references,
            limits.max_references(),
        ),
        (
            CanonicalValidationLimit::PayloadCells,
            evidence.counts.payload_cells,
            limits.max_payload_cells(),
        ),
        (
            CanonicalValidationLimit::MetadataEntries,
            evidence.counts.metadata_entries,
            limits.max_metadata_entries(),
        ),
        (
            CanonicalValidationLimit::MetadataBytes,
            evidence.counts.metadata_bytes,
            limits.max_metadata_bytes(),
        ),
        (
            CanonicalValidationLimit::CanonicalBytes,
            evidence.canonical_bytes,
            limits.max_canonical_bytes(),
        ),
        (
            CanonicalValidationLimit::ScratchBytes,
            evidence.scratch_peak_bytes,
            limits.max_scratch_bytes(),
        ),
    ];
    for (limit, actual, maximum) in checks {
        if actual > maximum {
            record_issue(
                issues,
                CanonicalValidationIssue::LimitExceeded {
                    limit,
                    actual,
                    maximum,
                },
            )?;
        }
    }
    Ok(())
}

fn record_issue(
    issues: &mut IssueCollector,
    issue: CanonicalValidationIssue,
) -> Result<(), CanonicalValidationError> {
    issues.record(issue)
}

fn empty_scratch_capacity_plan<T>(count: usize) -> Result<CapacityPlan, CanonicalValidationError> {
    CapacityPlan::for_type::<T>(0, 0, count)
        .map_err(|_| CanonicalValidationError::AccountingOverflow)
}

fn scratch_capacity_envelope(plan: CapacityPlan) -> Result<u64, CanonicalValidationError> {
    plan.envelope_delta_bytes()
        .map_err(|_| CanonicalValidationError::AccountingOverflow)
}

fn apply_empty_scratch_capacity_plan<T>(
    plan: CapacityPlan,
) -> Result<(Vec<T>, u64), CanonicalValidationError> {
    plan.verify_preflight(0, 0)
        .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| CanonicalValidationError::ScratchAllocationFailed)?;
    let actual_bytes = plan
        .finish(values.len(), values.capacity())
        .map_err(|_| CanonicalValidationError::AccountingOverflow)?;
    Ok((values, actual_bytes))
}

fn initialize_scratch_vec<T, C>(
    values: &mut Vec<T>,
    count: usize,
    value: T,
    evidence: &mut ValidationEvidence<'_, C>,
) -> ControlledValidationResult<(), C>
where
    T: Clone,
    C: SceneValidationControl,
{
    while values.len() < count {
        let remaining = checked_len(count - values.len())?;
        let amount = remaining.min(evidence.work_until_poll());
        evidence.add_typed_work(SceneValidationWork::Traversal, amount)?;
        let amount =
            usize::try_from(amount).map_err(|_| CanonicalValidationError::AccountingOverflow)?;
        values.resize(values.len() + amount, value.clone());
    }
    Ok(())
}

fn checked_len(value: usize) -> Result<u64, CanonicalValidationError> {
    u64::try_from(value).map_err(|_| CanonicalValidationError::AccountingOverflow)
}

fn checked_sum_lengths(values: &[usize]) -> Result<u64, CanonicalValidationError> {
    values.iter().try_fold(0_u64, |sum, value| {
        sum.checked_add(checked_len(*value)?)
            .ok_or(CanonicalValidationError::AccountingOverflow)
    })
}

fn outer_vec_bytes<T>(values: &Vec<T>) -> Result<usize, CanonicalValidationError> {
    values
        .capacity()
        .checked_mul(size_of::<T>())
        .ok_or(CanonicalValidationError::AccountingOverflow)
}

fn checked_add_bytes(left: usize, right: usize) -> Result<usize, CanonicalValidationError> {
    left.checked_add(right)
        .ok_or(CanonicalValidationError::AccountingOverflow)
}

fn checked_add_u64(left: u64, right: u64) -> Result<u64, CanonicalValidationError> {
    left.checked_add(right)
        .ok_or(CanonicalValidationError::AccountingOverflow)
}

fn issue_capacity_bytes(capacity: usize) -> Result<u64, CanonicalValidationError> {
    let bytes = capacity
        .checked_mul(size_of::<CanonicalValidationIssue>())
        .ok_or(CanonicalValidationError::AccountingOverflow)?;
    u64::try_from(bytes).map_err(|_| CanonicalValidationError::AccountingOverflow)
}

struct TraversalFrame {
    node: usize,
    next_child: usize,
    depth: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{
        AnimationChannel, AnimationClip, AnimationExtrapolation, AnimationInterpolation,
        AnimationLimits, AnimationTarget, AnimationTimeDomain, AnimationTimingMetadata,
        AnimationValues,
    };
    use crate::appearance::{
        AlphaMode, CanonicalMaterialDescriptor, ColorInterpretation, Material, MaterialFactors,
        ShadingModel, TextureBinding, TextureBlendOperation, TextureChannelMapping,
        TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureStrength,
        TextureTransform,
    };
    use crate::attribute::{
        Attribute, AttributeBuffer, AttributeDomain, AttributeFormat, AttributePayload,
        AttributeSemantic, ComponentType, CustomAttributeSemantic, Normalization,
        RegisteredPackedEncoding,
    };
    use crate::canonical_scene::Node;
    use crate::canonical_scene::SceneBounds;
    use crate::canonical_scene::builder::CanonicalSceneBuilder;
    use crate::deform::{
        CanonicalSkinDescriptor, Influence, InfluenceRows, MorphDelta, MorphTargetDefinition,
        PrimitiveMorphTarget, Skin, SkinMatrixSpaces,
    };
    use crate::derived::{Derived, DerivedAbsenceReason, DerivedProducer, InputStamp};
    use crate::derived_input::{
        ControlledDerivedInputStampError, logical_mesh_bounds_input_stamp,
        primitive_bounds_input_stamp, primitive_bounds_input_stamp_controlled,
        scene_bounds_input_stamp,
    };
    use crate::entity::AuthoredTransform;
    use crate::extension::{
        ExtensionBehaviorKind, ExtensionCardinality, ExtensionEntry, ExtensionEntryClassification,
        ExtensionInvalidationRule, ExtensionReferenceRule, ExtensionRemapRule, ExtensionSchema,
        ExtensionSchemaBehavior, ExtensionSchemaError, ExtensionSchemaId, ExtensionSchemaOwner,
        ExtensionSchemaSnapshot, ExtensionValue, ExtensionValueSchema,
        PackedAttributeEncodingSchema, PackedByteOrder,
    };
    use crate::geometry::{
        CanonicalMeshInstanceDescriptor, IndexStream, LogicalMesh, LogicalMeshBounds, MeshInstance,
        MeshPrimitive, MorphBoundsPolicy, PrimitiveBounds, Topology,
    };
    use crate::identity::{
        CameraIndex, ImageIndex, LightIndex, LogicalMeshIndex, MaterialIndex, MeshInstanceIndex,
        MorphTargetLocalIndex, NodeIndex, SamplerIndex, SkinIndex, TextureIndex,
    };
    use crate::image::{Image, Texture};
    use crate::math::{Aabb, Mat4, Quat, Vec3};
    use crate::namespaced::NamespacedName;
    use crate::provenance::{
        OperationEvidenceSummary, OperationIdentity, OperationKind, OptionsDigest, Provenance,
        ProvenanceArenaBuilder, ProvenanceAttachments, ResourceIdentity, SourceIdentity,
        SourceLocatorValue, SourceObjectLocator, test_provenance_fixture,
        test_provenance_pair_fixture,
    };
    use crate::scene_space::SceneSpace;
    use std::cell::Cell;
    use std::fmt;
    use std::rc::Rc;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestControlError {
        Cancelled,
        ReservationRejected,
        ReservationKindMismatch,
        ResizeRejected,
    }

    impl fmt::Display for TestControlError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(match self {
                Self::Cancelled => "cancelled by test control",
                Self::ReservationRejected => "reservation rejected by test control",
                Self::ReservationKindMismatch => "reservation kind mismatch",
                Self::ResizeRejected => "reservation resize rejected by test control",
            })
        }
    }

    impl std::error::Error for TestControlError {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestControlEvent {
        Poll,
        Work(SceneValidationWork, u64),
        Reserve(SceneReservationKind, u64),
        Resize(SceneReservationKind, u64),
    }

    struct TestReservation {
        kind: SceneReservationKind,
        bytes: u64,
        live_bytes: Rc<Cell<u64>>,
    }

    impl Drop for TestReservation {
        fn drop(&mut self) {
            self.live_bytes.set(
                self.live_bytes
                    .get()
                    .checked_sub(self.bytes)
                    .expect("reservation bytes must remain live until drop"),
            );
        }
    }

    struct SceneDropProbe(Rc<Cell<u8>>);

    impl Drop for SceneDropProbe {
        fn drop(&mut self) {
            assert_eq!(self.0.replace(1), 0, "scene must be released first");
        }
    }

    struct ReservationDropProbe(Rc<Cell<u8>>);

    impl Drop for ReservationDropProbe {
        fn drop(&mut self) {
            assert_eq!(
                self.0.replace(2),
                1,
                "reservation must remain live until scene release"
            );
        }
    }

    struct TestControl {
        events: Vec<TestControlEvent>,
        polls: usize,
        cancel_on_poll: Option<usize>,
        work: u64,
        live_bytes: Rc<Cell<u64>>,
        live_bytes_at_cancel: Option<u64>,
        reject_reservation: Option<SceneReservationKind>,
        reject_resize: bool,
    }

    impl TestControl {
        fn new() -> Self {
            Self {
                events: Vec::new(),
                polls: 0,
                cancel_on_poll: None,
                work: 0,
                live_bytes: Rc::new(Cell::new(0)),
                live_bytes_at_cancel: None,
                reject_reservation: None,
                reject_resize: false,
            }
        }

        fn cancel_on_poll(poll: usize) -> Self {
            let mut control = Self::new();
            control.cancel_on_poll = Some(poll);
            control
        }

        fn rejecting_reservation(kind: SceneReservationKind) -> Self {
            let mut control = Self::new();
            control.reject_reservation = Some(kind);
            control
        }

        fn rejecting_resize() -> Self {
            let mut control = Self::new();
            control.reject_resize = true;
            control
        }

        fn live_bytes(&self) -> u64 {
            self.live_bytes.get()
        }

        fn reserved(&self, kind: SceneReservationKind) -> bool {
            self.events.iter().any(
                |event| matches!(event, TestControlEvent::Reserve(actual, _) if *actual == kind),
            )
        }
    }

    impl SceneValidationControl for TestControl {
        type Error = TestControlError;
        type Reservation = TestReservation;

        fn poll(&mut self) -> Result<(), Self::Error> {
            self.events.push(TestControlEvent::Poll);
            self.polls += 1;
            if self.cancel_on_poll == Some(self.polls) {
                self.live_bytes_at_cancel = Some(self.live_bytes());
                Err(TestControlError::Cancelled)
            } else {
                Ok(())
            }
        }

        fn charge_work(
            &mut self,
            kind: SceneValidationWork,
            amount: u64,
        ) -> Result<(), Self::Error> {
            self.events.push(TestControlEvent::Work(kind, amount));
            self.work = self
                .work
                .checked_add(amount)
                .expect("test work accounting must not overflow");
            Ok(())
        }

        fn reserve(
            &mut self,
            kind: SceneReservationKind,
            bytes: u64,
        ) -> Result<Self::Reservation, Self::Error> {
            self.events.push(TestControlEvent::Reserve(kind, bytes));
            if self.reject_reservation == Some(kind) {
                return Err(TestControlError::ReservationRejected);
            }
            self.live_bytes.set(
                self.live_bytes()
                    .checked_add(bytes)
                    .expect("test reservation accounting must not overflow"),
            );
            Ok(TestReservation {
                kind,
                bytes,
                live_bytes: Rc::clone(&self.live_bytes),
            })
        }

        fn validate_reservation(
            &self,
            reservation: &Self::Reservation,
            kind: SceneReservationKind,
        ) -> Result<(), Self::Error> {
            if reservation.kind != kind {
                return Err(TestControlError::ReservationKindMismatch);
            }
            Ok(())
        }

        fn resize_reservation(
            &mut self,
            reservation: &mut Self::Reservation,
            kind: SceneReservationKind,
            required_bytes: u64,
        ) -> Result<(), Self::Error> {
            assert_eq!(reservation.kind, kind);
            self.events
                .push(TestControlEvent::Resize(kind, required_bytes));
            if self.reject_resize {
                return Err(TestControlError::ResizeRejected);
            }
            let without_old = self
                .live_bytes()
                .checked_sub(reservation.bytes)
                .expect("resized reservation must remain live");
            self.live_bytes.set(
                without_old
                    .checked_add(required_bytes)
                    .expect("test reservation accounting must not overflow"),
            );
            reservation.bytes = required_bytes;
            Ok(())
        }

        fn transition_reservation(
            &mut self,
            reservation: &mut Self::Reservation,
            from: SceneReservationKind,
            to: SceneReservationKind,
            required_bytes: u64,
        ) -> Result<(), Self::Error> {
            assert_eq!(reservation.kind, from);
            self.resize_reservation(reservation, from, required_bytes)?;
            reservation.kind = to;
            Ok(())
        }
    }

    fn node(provenance: Provenance, children: impl IntoIterator<Item = NodeIndex>) -> Node {
        Node::try_new_unchecked_references_for_test(
            None,
            AuthoredTransform::identity_trs(),
            children,
            None,
            None,
            provenance,
            [],
        )
        .expect("node fixture should be valid")
    }

    fn draft_with_children(children: &[&[u32]]) -> CanonicalSceneDraft {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene generation should remain available");
        for child_indexes in children {
            builder
                .try_push_node_unchecked_for_test(node(
                    provenance,
                    child_indexes.iter().copied().map(NodeIndex::from_u32),
                ))
                .expect("node allocation should succeed");
        }
        builder.set_root_unchecked_for_test(NodeIndex::from_u32(0));
        builder.finish_draft()
    }

    fn chain_draft(node_count: usize) -> CanonicalSceneDraft {
        assert!(node_count > 0);
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene generation should remain available");
        for index in 0..node_count {
            let children = if index + 1 < node_count {
                vec![NodeIndex::from_u32(
                    u32::try_from(index + 1).expect("test node index should fit"),
                )]
            } else {
                Vec::new()
            };
            builder
                .try_push_node_unchecked_for_test(node(provenance, children))
                .expect("node allocation should succeed");
        }
        builder.set_root_unchecked_for_test(NodeIndex::from_u32(0));
        builder.finish_draft()
    }

    fn issue_set(error: &CanonicalValidationError) -> &[CanonicalValidationIssue] {
        error.issues()
    }

    fn positions(count: usize, provenance: Provenance) -> Attribute {
        let format = AttributeFormat::new(ComponentType::F32, 3, Normalization::None, None)
            .expect("plain position format should be valid");
        Attribute::new(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            format,
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO; count]).unwrap(),
            ),
            provenance,
        )
        .unwrap()
    }

    fn positions_with_values(values: Vec<Vec3>, provenance: Provenance) -> Attribute {
        let format = AttributeFormat::new(ComponentType::F32, 3, Normalization::None, None)
            .expect("plain position format should be valid");
        Attribute::new(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            format,
            AttributePayload::dense(AttributeBuffer::from_f32_vec3(values).unwrap()),
            provenance,
        )
        .unwrap()
    }

    fn bounds_fixture_primitive(
        provenance: Provenance,
        values: Vec<Vec3>,
        topology: Topology,
        bounds: PrimitiveBounds,
    ) -> MeshPrimitive {
        let vertex_count = u32::try_from(values.len()).expect("fixture vertex count should fit");
        MeshPrimitive::try_new(
            vertex_count,
            topology,
            IndexStream::non_indexed(),
            None,
            vec![positions_with_values(values, provenance)],
            None,
            Vec::new(),
            0,
            bounds,
            provenance,
        )
        .expect("bounds fixture primitive should be valid")
    }

    fn bounds_fixture_primitive_with_morph(
        provenance: Provenance,
        delta_y: f32,
        policy: MorphBoundsPolicy,
    ) -> MeshPrimitive {
        let delta = MorphDelta::try_new(
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec3(vec![
                    Vec3::ZERO,
                    Vec3::new(0.0, delta_y, 0.0),
                    Vec3::ZERO,
                ])
                .unwrap(),
            ),
            provenance,
        )
        .unwrap();
        let target = PrimitiveMorphTarget::try_new(
            MorphTargetLocalIndex::from_u32(0),
            3,
            [(MorphDeltaSemantic::Position, delta)],
        )
        .unwrap();
        MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            vec![positions_with_values(
                bounds_fixture_positions(),
                provenance,
            )],
            None,
            vec![target],
            1,
            PrimitiveBounds::absent(DerivedAbsenceReason::NotProvided, policy),
            provenance,
        )
        .unwrap()
    }

    const BOUNDS_FIXTURE_AABB: Aabb = Aabb {
        min: Vec3::ZERO,
        max: Vec3::new(1.0, 1.0, 0.0),
    };

    fn bounds_fixture_positions() -> Vec<Vec3> {
        vec![
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ]
    }

    fn behavior_schema(
        schema_id: ExtensionSchemaId,
        owner: ExtensionSchemaOwner,
        behavior: ExtensionSchemaBehavior,
    ) -> ExtensionSchema {
        ExtensionSchema::new_with_behavior(
            schema_id,
            owner,
            ExtensionValueSchema::Bool,
            behavior,
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::OnOwnerChange,
        )
        .unwrap()
    }

    fn source_locator_schema(schema_id: ExtensionSchemaId, maximum_bytes: u32) -> ExtensionSchema {
        ExtensionSchema::new_with_behavior(
            schema_id,
            ExtensionSchemaOwner::SourceIdentity,
            ExtensionValueSchema::String {
                max_bytes: maximum_bytes,
            },
            ExtensionSchemaBehavior::Canonical(ExtensionBehaviorKind::SourceObjectLocator),
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::Never,
        )
        .expect("source locator schema fixture should match the closed contract")
    }

    fn draft_with_namespaced_source_locator(
        locator_schema_id: ExtensionSchemaId,
        locator_value: &str,
        schemas: ExtensionSchemaSnapshot,
    ) -> CanonicalSceneDraft {
        let mut provenance_builder =
            ProvenanceArenaBuilder::try_new().expect("test arena nonce should remain available");
        let provenance = provenance_builder
            .add_source(SourceIdentity::new(
                ResourceIdentity::new("tests/namespaced-locator.asset").unwrap(),
                crate::FormatId::new("dev.baozi.test").unwrap(),
                SourceObjectLocator::try_namespaced(
                    Some(locator_schema_id),
                    SourceLocatorValue::new(locator_value).unwrap(),
                )
                .unwrap(),
            ))
            .unwrap();
        let arena = provenance_builder.finish().unwrap();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            schemas,
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        builder.finish_draft()
    }

    fn draft_with_shared_skinned_instances(
        influence_count: usize,
        instance_count: usize,
    ) -> CanonicalSceneDraft {
        assert!(influence_count > 0);
        assert!(instance_count > 0);
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        let root_id = root.handle(builder.generation());
        let influence_count_u32 =
            u32::try_from(influence_count).expect("test influence count should fit");
        let influences = InfluenceRows::try_new(
            1,
            [0, influence_count_u32],
            (0..influence_count).map(|source_order| {
                Influence::new(
                    0,
                    1.0,
                    u32::try_from(source_order).expect("test source order should fit"),
                )
            }),
        )
        .unwrap();
        let primitive = MeshPrimitive::try_new(
            1,
            Topology::Points,
            IndexStream::non_indexed(),
            None,
            vec![positions(1, provenance)],
            Some(influences),
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        )
        .unwrap();
        let mesh = builder
            .try_push_logical_mesh(
                LogicalMesh::try_new(
                    None,
                    vec![primitive],
                    Vec::new(),
                    Vec::new(),
                    LogicalMeshBounds::default(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        let skin_id = builder
            .try_push_skin(
                CanonicalSkinDescriptor::try_new(
                    None,
                    [root_id],
                    [Mat4::IDENTITY],
                    None,
                    SkinMatrixSpaces::canonical(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        for _ in 0..instance_count {
            builder
                .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
                    root_id,
                    mesh,
                    Some(skin_id),
                    None,
                    Vec::new(),
                    provenance,
                ))
                .unwrap();
        }
        builder.finish_draft()
    }

    fn draft_with_namespaced_shading_schema(
        schema_id: ExtensionSchemaId,
        schemas: ExtensionSchemaSnapshot,
    ) -> CanonicalSceneDraft {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            schemas,
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        builder
            .try_push_material(
                CanonicalMaterialDescriptor::try_new(
                    None,
                    ShadingModel::try_namespaced(Some(schema_id)).unwrap(),
                    AlphaMode::Opaque,
                    false,
                    MaterialFactors::empty(),
                    [],
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        builder.finish_draft()
    }

    fn packed_schema(schema_id: ExtensionSchemaId, byte_order: PackedByteOrder) -> ExtensionSchema {
        let encoding =
            PackedAttributeEncodingSchema::octahedral_unit_vector_snorm16(byte_order).unwrap();
        ExtensionSchema::new_with_behavior(
            schema_id,
            ExtensionSchemaOwner::Attribute,
            ExtensionValueSchema::Vec3,
            ExtensionSchemaBehavior::PackedAttribute(encoding),
            ExtensionCardinality::PerAttributeDomain(AttributeDomain::Vertex),
            ExtensionReferenceRule::None,
            ExtensionRemapRule::RemapCardinality,
            ExtensionInvalidationRule::OnOwnerChange,
        )
        .unwrap()
    }

    fn primitive_with_material(material: MaterialIndex, provenance: Provenance) -> MeshPrimitive {
        MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            Some(material),
            vec![positions(3, provenance)],
            None,
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        )
        .unwrap()
    }

    fn mesh_with_influences_and_two_morphs(provenance: Provenance) -> LogicalMesh {
        let influences =
            InfluenceRows::try_new(3, [0, 1, 1, 1], [Influence::new(1, 1.0, 0)]).unwrap();
        let primitive = MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            vec![positions(3, provenance)],
            Some(influences),
            Vec::new(),
            2,
            PrimitiveBounds::default(),
            provenance,
        )
        .unwrap();
        LogicalMesh::try_new(
            None,
            vec![primitive],
            vec![
                MorphTargetDefinition::try_new(None, provenance).unwrap(),
                MorphTargetDefinition::try_new(None, provenance).unwrap(),
            ],
            vec![0.0, 0.0],
            LogicalMeshBounds::default(),
            provenance,
        )
        .unwrap()
    }

    fn material_with_unchecked_texture(texture: TextureIndex, provenance: Provenance) -> Material {
        let binding = TextureBinding::try_new_unchecked_reference_for_test(
            texture,
            TextureRole::BaseColor,
            TextureCoordinateSource::uv_set(0).unwrap(),
            TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA),
            ColorInterpretation::SrgbColor,
            TextureTransform::IDENTITY,
            TextureStrength::ONE,
            TextureBlendOperation::Replace,
            None,
            provenance,
            [],
        )
        .unwrap();
        Material::try_new_unchecked_references_for_test(
            None,
            ShadingModel::Unlit,
            AlphaMode::Opaque,
            false,
            MaterialFactors::empty(),
            [binding],
            provenance,
            [],
        )
        .unwrap()
    }

    fn translation_channel(node: NodeIndex, provenance: Provenance) -> AnimationChannel {
        AnimationChannel::try_new(
            AnimationTarget::NodeTranslation(node),
            AnimationInterpolation::Linear,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            AnimationValues::try_translations([Vec3::ZERO], &AnimationLimits::canonical()).unwrap(),
            AnimationTimingMetadata::empty(),
            provenance,
            &AnimationLimits::canonical(),
        )
        .unwrap()
    }

    fn scale_channel(node: NodeIndex, provenance: Provenance) -> AnimationChannel {
        AnimationChannel::try_new(
            AnimationTarget::NodeScale(node),
            AnimationInterpolation::Linear,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            AnimationValues::try_scales([Vec3::new(1.0, 1.0, 1.0)], &AnimationLimits::canonical())
                .unwrap(),
            AnimationTimingMetadata::empty(),
            provenance,
            &AnimationLimits::canonical(),
        )
        .unwrap()
    }

    fn morph_channel(instance: MeshInstanceIndex, provenance: Provenance) -> AnimationChannel {
        AnimationChannel::try_new(
            AnimationTarget::MeshInstanceMorphWeights(instance),
            AnimationInterpolation::Linear,
            [0.0],
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            AnimationValues::try_morph_weights(1, [vec![0.5]], &AnimationLimits::canonical())
                .unwrap(),
            AnimationTimingMetadata::empty(),
            provenance,
            &AnimationLimits::canonical(),
        )
        .unwrap()
    }

    #[test]
    fn valid_deep_tree_reports_exact_graph_shape() {
        let scene = validate_canonical_scene(
            draft_with_children(&[&[1], &[2], &[]]),
            SceneValidationLimits::default(),
        )
        .expect("deep tree should validate");
        let stats = scene.validation_stats();

        assert_eq!(stats.canonical_counts().nodes, 3);
        assert_eq!(stats.canonical_counts().edges, 2);
        assert_eq!(stats.maximum_depth(), 3);
    }

    #[test]
    fn encoded_images_report_exact_logical_byte_counts() {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene generation should remain available");
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .expect("root node allocation should succeed");
        builder.set_root_unchecked_for_test(root);
        builder
            .try_push_image(
                Image::try_encoded([1, 2, 3], None, None, None, None, provenance)
                    .expect("first encoded image should be valid"),
            )
            .expect("first image allocation should succeed");
        builder
            .try_push_image(
                Image::try_encoded([4, 5, 6, 7, 8], None, None, None, None, provenance)
                    .expect("second encoded image should be valid"),
            )
            .expect("second image allocation should succeed");

        let scene =
            validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                .expect("encoded images should validate");
        let counts = scene.validation_stats().canonical_counts();

        assert_eq!(counts.embedded_bytes, 8);
        assert_eq!(counts.payload_cells, 8);
    }

    #[test]
    fn controlled_validation_charges_exact_work_with_bounded_poll_windows() {
        let mut control = TestControl::new();
        let success = validate_canonical_scene_with(
            chain_draft(SCENE_VALIDATION_POLL_INTERVAL as usize + 17),
            SceneValidationLimits::default(),
            &mut control,
        )
        .unwrap_or_else(|error| panic!("controlled validation should succeed: {error:?}"));
        let stats = success.stats();

        assert_eq!(control.work, stats.work_units());
        let mut work_since_poll = 0;
        for event in &control.events {
            match event {
                TestControlEvent::Poll => work_since_poll = 0,
                TestControlEvent::Work(_, amount) => {
                    work_since_poll += amount;
                    assert!(work_since_poll <= SCENE_VALIDATION_POLL_INTERVAL);
                }
                TestControlEvent::Reserve(_, _) | TestControlEvent::Resize(_, _) => {}
            }
        }
        assert!(control.reserved(SceneReservationKind::ValidationScratch));
        assert!(control.reserved(SceneReservationKind::CanonicalOutput));
        assert_eq!(
            control.live_bytes(),
            stats.footprint().accounted_payload_bytes()
        );

        drop(success);
        assert_eq!(control.live_bytes(), 0);
    }

    #[test]
    fn max_work_is_inclusive_and_rejects_before_the_next_unit() {
        let baseline = validate_canonical_scene(
            draft_with_children(&[&[1], &[]]),
            SceneValidationLimits::default(),
        )
        .expect("baseline scene should validate");
        let exact_work = baseline.validation_stats().work_units();
        assert!(exact_work > 0);

        validate_canonical_scene(
            draft_with_children(&[&[1], &[]]),
            SceneValidationLimits::default().with_max_work(exact_work),
        )
        .expect("the exact work boundary must be inclusive");

        let mut control = TestControl::new();
        let error = validate_canonical_scene_with(
            draft_with_children(&[&[1], &[]]),
            SceneValidationLimits::default().with_max_work(exact_work - 1),
            &mut control,
        )
        .err()
        .expect("one work unit below the observed total must fail");
        assert!(matches!(
            error,
            CanonicalValidationFailure::Validation(
                CanonicalValidationError::WorkLimitExceeded { actual, maximum }
            ) if actual == exact_work && maximum == exact_work - 1
        ));
        assert_eq!(control.work, exact_work - 1);
        assert!(!control.reserved(SceneReservationKind::CanonicalOutput));
        assert_eq!(control.live_bytes(), 0);
    }

    #[test]
    fn top_level_limits_and_zero_diagnostic_budgets_fail_before_expensive_passes() {
        let mut control = TestControl::new();
        let error = validate_canonical_scene_with(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default()
                .with_max_nodes(0)
                .with_max_diagnostic_issues(0)
                .with_max_diagnostic_bytes(0),
            &mut control,
        )
        .err()
        .expect("the top-level node limit must fail closed");
        match error {
            CanonicalValidationFailure::Validation(CanonicalValidationError::Invalid {
                issues,
                omitted_issue_count,
                retained_issue_bytes,
            }) => {
                assert!(issues.is_empty());
                assert_eq!(omitted_issue_count, 1);
                assert_eq!(retained_issue_bytes, 0);
            }
            other => panic!("unexpected top-level limit failure: {other:?}"),
        }
        assert_eq!(control.work, 0);
        assert_eq!(control.polls, 1);
        assert!(!control.reserved(SceneReservationKind::ValidationScratch));
        assert!(!control.reserved(SceneReservationKind::CanonicalOutput));
    }

    #[test]
    fn scratch_limit_is_inclusive_and_rejects_before_reservation() {
        let baseline = validate_canonical_scene(
            draft_with_children(&[&[1], &[]]),
            SceneValidationLimits::default(),
        )
        .expect("baseline hierarchy should validate");
        let scratch_bytes = baseline.validation_stats().scratch_peak_bytes();
        assert!(scratch_bytes > 0);

        let mut exact_control = TestControl::new();
        let exact = validate_canonical_scene_with(
            draft_with_children(&[&[1], &[]]),
            SceneValidationLimits::default().with_max_scratch_bytes(scratch_bytes),
            &mut exact_control,
        )
        .unwrap_or_else(|error| panic!("exact scratch boundary should succeed: {error:?}"));
        assert!(exact_control.events.iter().any(|event| matches!(
            event,
            TestControlEvent::Reserve(SceneReservationKind::ValidationScratch, bytes)
                if *bytes == scratch_bytes
        )));
        drop(exact);
        assert_eq!(exact_control.live_bytes(), 0);

        let mut below_control =
            TestControl::rejecting_reservation(SceneReservationKind::ValidationScratch);
        let error = validate_canonical_scene_with(
            draft_with_children(&[&[1], &[]]),
            SceneValidationLimits::default().with_max_scratch_bytes(scratch_bytes - 1),
            &mut below_control,
        )
        .err()
        .expect("one byte below the scratch envelope must fail");
        match error {
            CanonicalValidationFailure::Validation(error) => assert!(error.issues().contains(
                &CanonicalValidationIssue::LimitExceeded {
                    limit: CanonicalValidationLimit::ScratchBytes,
                    actual: scratch_bytes,
                    maximum: scratch_bytes - 1,
                }
            )),
            CanonicalValidationFailure::Control(error) => {
                panic!("scratch limit must fail before control reservation: {error:?}")
            }
        }
        assert!(!below_control.reserved(SceneReservationKind::ValidationScratch));
        assert!(!below_control.reserved(SceneReservationKind::CanonicalOutput));
        assert_eq!(below_control.live_bytes(), 0);
    }

    #[test]
    fn scratch_control_failures_release_tokens_without_canonical_output() {
        let mut reserve_control =
            TestControl::rejecting_reservation(SceneReservationKind::ValidationScratch);
        let reserve_error = validate_canonical_scene_with(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default(),
            &mut reserve_control,
        )
        .err()
        .expect("scratch reservation rejection must fail validation");
        assert!(matches!(
            reserve_error,
            CanonicalValidationFailure::Control(TestControlError::ReservationRejected)
        ));
        assert!(!reserve_control.reserved(SceneReservationKind::CanonicalOutput));
        assert_eq!(reserve_control.live_bytes(), 0);

        let mut resize_control = TestControl::rejecting_resize();
        let resize_error = validate_canonical_scene_with(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default(),
            &mut resize_control,
        )
        .err()
        .expect("scratch resize rejection must fail validation");
        assert!(matches!(
            resize_error,
            CanonicalValidationFailure::Control(TestControlError::ResizeRejected)
        ));
        assert!(!resize_control.reserved(SceneReservationKind::CanonicalOutput));
        assert_eq!(resize_control.live_bytes(), 0);
    }

    #[test]
    fn cancellation_never_leaks_or_publishes_an_invalid_canonical_scene() {
        let mut immediate_control = TestControl::cancel_on_poll(1);
        let immediate_error = validate_canonical_scene_with(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default(),
            &mut immediate_control,
        )
        .err()
        .expect("the first cancellation poll must stop validation");
        assert!(matches!(
            immediate_error,
            CanonicalValidationFailure::Control(TestControlError::Cancelled)
        ));
        assert_eq!(immediate_control.work, 0);
        assert!(
            immediate_control
                .events
                .iter()
                .all(|event| matches!(event, TestControlEvent::Poll))
        );

        let mut baseline_control = TestControl::new();
        let baseline = validate_canonical_scene_with(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default(),
            &mut baseline_control,
        )
        .unwrap_or_else(|error| panic!("baseline validation should succeed: {error:?}"));
        let final_poll = baseline_control.polls;
        drop(baseline);
        assert_eq!(baseline_control.live_bytes(), 0);

        let mut final_control = TestControl::cancel_on_poll(final_poll);
        let final_error = validate_canonical_scene_with(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default(),
            &mut final_control,
        )
        .err()
        .expect("cancellation after canonical reservation must fail validation");
        assert!(matches!(
            final_error,
            CanonicalValidationFailure::Control(TestControlError::Cancelled)
        ));
        assert!(final_control.reserved(SceneReservationKind::CanonicalOutput));
        assert!(
            final_control
                .live_bytes_at_cancel
                .is_some_and(|bytes| bytes > 0)
        );
        assert_eq!(final_control.live_bytes(), 0);

        let mut invalid_control = TestControl::new();
        let invalid_error = validate_canonical_scene_with(
            draft_with_children(&[&[99]]),
            SceneValidationLimits::default(),
            &mut invalid_control,
        )
        .err()
        .expect("an invalid hierarchy must fail validation");
        assert!(matches!(
            invalid_error,
            CanonicalValidationFailure::Validation(CanonicalValidationError::Invalid { .. })
        ));
        assert!(!invalid_control.reserved(SceneReservationKind::CanonicalOutput));
        assert_eq!(invalid_control.live_bytes(), 0);
    }

    #[test]
    fn cancellation_while_hierarchy_scratch_is_live_releases_the_token() {
        let node_count = SCENE_VALIDATION_POLL_INTERVAL as usize + 17;
        let mut baseline_control = TestControl::new();
        let baseline = validate_canonical_scene_with(
            chain_draft(node_count),
            SceneValidationLimits::default(),
            &mut baseline_control,
        )
        .expect("baseline hierarchy should validate");
        drop(baseline);

        let scratch_reserve = baseline_control
            .events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    TestControlEvent::Reserve(SceneReservationKind::ValidationScratch, _)
                )
            })
            .expect("hierarchy validation must reserve scratch");
        let cancel_poll = baseline_control.events[..=scratch_reserve]
            .iter()
            .filter(|event| matches!(event, TestControlEvent::Poll))
            .count()
            + 1;

        let mut control = TestControl::cancel_on_poll(cancel_poll);
        let error = validate_canonical_scene_with(
            chain_draft(node_count),
            SceneValidationLimits::default(),
            &mut control,
        )
        .err()
        .expect("a poll while hierarchy scratch is live must cancel");
        assert!(matches!(
            error,
            CanonicalValidationFailure::Control(TestControlError::Cancelled)
        ));
        assert!(control.live_bytes_at_cancel.is_some_and(|bytes| bytes > 0));
        assert!(control.reserved(SceneReservationKind::ValidationScratch));
        assert!(!control.reserved(SceneReservationKind::CanonicalOutput));
        assert_eq!(control.live_bytes(), 0);
    }

    #[test]
    fn publication_failure_consumes_the_success_and_releases_its_reservation() {
        let mut control = TestControl::new();
        let success = validate_canonical_scene_with(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("valid input should reach the publication boundary");
        assert!(control.live_bytes() > 0);

        let error = match success.try_publish(|_reservation| Err("publication rejected")) {
            Ok(_) => panic!("publication failure must not expose the canonical scene"),
            Err(error) => error,
        };
        assert_eq!(error, "publication rejected");
        assert_eq!(control.live_bytes(), 0);
    }

    #[test]
    fn publication_success_returns_the_canonical_scene_with_its_companion() {
        let mut control = TestControl::new();
        let success = validate_canonical_scene_with(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("valid input should reach the publication boundary");
        assert!(control.live_bytes() > 0);

        let (scene, companion) = success
            .try_publish_with_companion(
                vec!["staged"],
                |_reservation, companion| {
                    companion.push("published");
                    Ok::<(), ()>(())
                },
                |_error, _companion| unreachable!("publication should succeed"),
            )
            .expect("publication should return scene and companion together");

        assert_eq!(scene.validation_stats().canonical_counts().nodes, 1);
        assert_eq!(companion, ["staged", "published"]);
        assert_eq!(control.live_bytes(), 0);
    }

    #[test]
    fn publication_failure_finalizes_the_canonical_companion_before_rollback() {
        let mut control = TestControl::new();
        let success = validate_canonical_scene_with(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("valid input should reach the publication boundary");
        assert!(control.live_bytes() > 0);

        let failure = match success.try_publish_with_companion(
            vec!["staged"],
            |_reservation, companion| {
                companion.push("attempted");
                Err("publication rejected")
            },
            |error, companion| (error, companion),
        ) {
            Ok(_) => panic!("publication failure must not expose the canonical scene"),
            Err(failure) => failure,
        };

        assert_eq!(failure.0, "publication rejected");
        assert_eq!(failure.1, ["staged", "attempted"]);
        assert_eq!(control.live_bytes(), 0);
    }

    #[test]
    fn publication_failure_drops_scene_before_rollback_reservation() {
        let state = Rc::new(Cell::new(0));
        let error = try_publish_coupled(
            SceneDropProbe(Rc::clone(&state)),
            ReservationDropProbe(Rc::clone(&state)),
            |_reservation| Err("rejected"),
        )
        .err()
        .expect("publication should be rejected");

        assert_eq!(error, "rejected");
        assert_eq!(state.get(), 2);
    }

    #[test]
    fn publication_panic_drops_scene_before_rollback_reservation() {
        let state = Rc::new(Cell::new(0));
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<SceneDropProbe, ()> = try_publish_coupled(
                SceneDropProbe(Rc::clone(&state)),
                ReservationDropProbe(Rc::clone(&state)),
                |_reservation| panic!("publication panic fixture"),
            );
        }));

        assert!(unwind.is_err());
        assert_eq!(state.get(), 2);
    }

    #[test]
    fn disconnected_cycle_is_rejected_as_cycle_and_unreachable() {
        let error = validate_canonical_scene(
            draft_with_children(&[&[], &[2], &[1]]),
            SceneValidationLimits::default(),
        )
        .err()
        .expect("disconnected cycle must fail validation");

        assert!(issue_set(&error).contains(&CanonicalValidationIssue::HierarchyCycle { node: 1 }));
        assert!(issue_set(&error).contains(&CanonicalValidationIssue::UnreachableNode { node: 1 }));
        assert!(issue_set(&error).contains(&CanonicalValidationIssue::UnreachableNode { node: 2 }));
    }

    #[test]
    fn shared_child_and_out_of_range_child_return_stable_issues() {
        let error = validate_canonical_scene(
            draft_with_children(&[&[1, 2, 99], &[2], &[]]),
            SceneValidationLimits::default(),
        )
        .err()
        .expect("shared or out-of-range children must fail validation");

        assert!(
            issue_set(&error)
                .contains(&CanonicalValidationIssue::ChildOutOfRange { node: 0, child: 99 })
        );
        assert!(
            issue_set(&error)
                .contains(&CanonicalValidationIssue::NodeParentCount { node: 2, actual: 2 })
        );
    }

    #[test]
    fn diagnostic_limits_retain_a_stable_prefix_at_exact_boundaries() {
        let build_draft = || draft_with_children(&[&[99, 100, 101, 102, 103]]);
        let baseline = validate_canonical_scene(build_draft(), SceneValidationLimits::default())
            .err()
            .expect("out-of-range children must fail validation");
        let full_issues = baseline.issues().to_vec();
        let full_issue_count = u64::try_from(full_issues.len()).unwrap();
        let actual_retained_issue_bytes = baseline.retained_issue_bytes();
        let full_admission_envelope_bytes = issue_capacity_bytes(
            CapacityPlan::for_type::<CanonicalValidationIssue>(0, 0, full_issues.len())
                .expect("the small diagnostic capacity plan must fit")
                .envelope_capacity(),
        )
        .expect("the small diagnostic admission envelope must fit");

        assert_eq!(full_issue_count, 5);
        assert_eq!(baseline.omitted_issue_count(), 0);
        assert!(actual_retained_issue_bytes > 0);
        assert!(actual_retained_issue_bytes <= full_admission_envelope_bytes);

        let exact = validate_canonical_scene(
            build_draft(),
            SceneValidationLimits::default()
                .with_max_diagnostic_issues(full_issue_count)
                .with_max_diagnostic_bytes(full_admission_envelope_bytes),
        )
        .err()
        .expect("the exact diagnostic boundaries must retain every issue");
        assert_eq!(exact.issues(), full_issues);
        assert_eq!(exact.omitted_issue_count(), 0);
        assert!(exact.retained_issue_bytes() <= full_admission_envelope_bytes);

        let prefix_len = full_issues.len() - 1;
        let issue_limited = validate_canonical_scene(
            build_draft(),
            SceneValidationLimits::default()
                .with_max_diagnostic_issues(full_issue_count - 1)
                .with_max_diagnostic_bytes(full_admission_envelope_bytes),
        )
        .err()
        .expect("the issue-count boundary must still report validation failure");
        assert_eq!(issue_limited.issues(), &full_issues[..prefix_len]);
        assert_eq!(issue_limited.omitted_issue_count(), 1);
        assert!(issue_limited.retained_issue_bytes() <= full_admission_envelope_bytes);

        let byte_limit = full_admission_envelope_bytes - 1;
        let byte_limited = validate_canonical_scene(
            build_draft(),
            SceneValidationLimits::default()
                .with_max_diagnostic_issues(full_issue_count)
                .with_max_diagnostic_bytes(byte_limit),
        )
        .err()
        .expect("the byte boundary must still report validation failure");
        assert_eq!(byte_limited.issues(), &full_issues[..prefix_len]);
        assert_eq!(byte_limited.omitted_issue_count(), 1);
        assert!(byte_limited.retained_issue_bytes() <= byte_limit);

        let zero_byte_limited = validate_canonical_scene(
            build_draft(),
            SceneValidationLimits::default()
                .with_max_diagnostic_issues(full_issue_count)
                .with_max_diagnostic_bytes(0),
        )
        .err()
        .expect("a zero byte budget must still report validation failure");
        assert!(zero_byte_limited.issues().is_empty());
        assert_eq!(zero_byte_limited.omitted_issue_count(), full_issue_count);
        assert_eq!(zero_byte_limited.retained_issue_bytes(), 0);
    }

    #[test]
    fn foreign_provenance_is_rejected_before_publication() {
        let (arena, scene_provenance) = test_provenance_fixture();
        let (_, foreign) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            scene_provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(foreign, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);

        let error =
            validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                .err()
                .expect("foreign provenance must fail validation");
        assert_eq!(
            issue_set(&error),
            &[CanonicalValidationIssue::ForeignProvenance {
                owner: CanonicalOwner::Entity {
                    kind: SceneEntityKind::Node,
                    index: 0,
                },
            }]
        );
    }

    #[test]
    fn scene_provenance_is_mandatory_accounted_and_arena_bound() {
        let (arena, provenance) = test_provenance_fixture();
        let (_, foreign) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            foreign,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        let error =
            validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                .err()
                .expect("foreign scene provenance must fail validation");
        assert_eq!(
            error.issues(),
            &[CanonicalValidationIssue::ForeignProvenance {
                owner: CanonicalOwner::Scene,
            }]
        );

        let scene_bytes = |multiple: bool| {
            let (arena, [first, second]) = test_provenance_pair_fixture();
            let scene_provenance = if multiple {
                ProvenanceAttachments::try_from_iter([first, second]).unwrap()
            } else {
                first.into()
            };
            let attachment_bytes = scene_provenance.retained_allocation_bytes();
            let mut builder = CanonicalSceneBuilder::try_new(
                SceneSpace::unknown(),
                ExtensionSchemaSnapshot::empty(),
                arena,
                scene_provenance,
                DerivedAbsenceReason::NotProvided,
            )
            .unwrap();
            let root = builder
                .try_push_node_unchecked_for_test(node(first, []))
                .unwrap();
            builder.set_root_unchecked_for_test(root);
            let scene =
                validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                    .unwrap();
            (
                scene
                    .validation_stats()
                    .footprint()
                    .accounted_payload_bytes(),
                u64::try_from(attachment_bytes).unwrap(),
            )
        };
        let (single_bytes, single_attachment_bytes) = scene_bytes(false);
        let (multiple_bytes, multiple_attachment_bytes) = scene_bytes(true);
        assert_eq!(single_attachment_bytes, 0);
        assert_eq!(
            multiple_bytes - single_bytes,
            multiple_attachment_bytes,
            "scene provenance attachment capacity must be part of canonical bytes"
        );
    }

    #[test]
    fn foreign_provenance_is_rejected_for_unreferenced_scene_entities() {
        let (arena, provenance) = test_provenance_fixture();
        let (_, foreign) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        builder
            .try_push_camera(
                crate::entity::Camera::try_new(
                    None,
                    crate::entity::CameraProjection::perspective(1.0, None, 0.1, None).unwrap(),
                    foreign,
                    [],
                )
                .unwrap(),
            )
            .unwrap();

        let error =
            validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                .err()
                .expect("every scene-owned entity provenance must be validated");

        assert_eq!(
            issue_set(&error),
            &[CanonicalValidationIssue::ForeignProvenance {
                owner: CanonicalOwner::Entity {
                    kind: SceneEntityKind::Camera,
                    index: 0,
                },
            }]
        );
    }

    #[test]
    fn namespaced_source_locators_require_a_closed_scene_snapshot_schema() {
        let schema_id = ExtensionSchemaId::new("test:source-locator", 2).unwrap();
        let valid_schema = source_locator_schema(schema_id.clone(), 4);
        validate_canonical_scene(
            draft_with_namespaced_source_locator(
                schema_id.clone(),
                "ship",
                ExtensionSchemaSnapshot::try_from_schemas([valid_schema]).unwrap(),
            ),
            SceneValidationLimits::default(),
        )
        .expect("an owned matching source-locator schema should validate independently");

        let missing = validate_canonical_scene(
            draft_with_namespaced_source_locator(
                schema_id.clone(),
                "ship",
                ExtensionSchemaSnapshot::empty(),
            ),
            SceneValidationLimits::default(),
        )
        .err()
        .expect("missing source locator schema should fail validation");
        assert_eq!(
            missing.issues(),
            &[CanonicalValidationIssue::ExtensionSchemaMissing {
                owner: CanonicalOwner::ProvenanceNode { index: 0 },
                behavior: ExtensionBehaviorKind::SourceObjectLocator,
                schema_id: schema_id.clone(),
            }]
        );

        let wrong_owner = ExtensionSchema::new(
            schema_id.clone(),
            ExtensionSchemaOwner::Scene,
            ExtensionValueSchema::String { max_bytes: 4 },
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::Never,
        )
        .unwrap();
        let wrong_owner = validate_canonical_scene(
            draft_with_namespaced_source_locator(
                schema_id.clone(),
                "ship",
                ExtensionSchemaSnapshot::try_from_schemas([wrong_owner]).unwrap(),
            ),
            SceneValidationLimits::default(),
        )
        .err()
        .expect("source locator schema with the wrong owner should fail validation");
        assert_eq!(
            wrong_owner.issues(),
            &[CanonicalValidationIssue::ExtensionSchemaOwnerMismatch {
                owner: CanonicalOwner::ProvenanceNode { index: 0 },
                behavior: ExtensionBehaviorKind::SourceObjectLocator,
                expected: ExtensionSchemaOwner::SourceIdentity,
                actual: ExtensionSchemaOwner::Scene,
            }]
        );

        let old_schema_id = ExtensionSchemaId::new("test:source-locator", 1).unwrap();
        let wrong_version = validate_canonical_scene(
            draft_with_namespaced_source_locator(
                schema_id.clone(),
                "ship",
                ExtensionSchemaSnapshot::try_from_schemas([source_locator_schema(
                    old_schema_id,
                    4,
                )])
                .unwrap(),
            ),
            SceneValidationLimits::default(),
        )
        .err()
        .expect("source locator schema with the wrong version should fail validation");
        assert_eq!(
            wrong_version.issues(),
            &[CanonicalValidationIssue::ExtensionSchemaMissing {
                owner: CanonicalOwner::ProvenanceNode { index: 0 },
                behavior: ExtensionBehaviorKind::SourceObjectLocator,
                schema_id: schema_id.clone(),
            }]
        );

        let oversized = validate_canonical_scene(
            draft_with_namespaced_source_locator(
                schema_id.clone(),
                "ship",
                ExtensionSchemaSnapshot::try_from_schemas([source_locator_schema(
                    schema_id.clone(),
                    3,
                )])
                .unwrap(),
            ),
            SceneValidationLimits::default(),
        )
        .err()
        .expect("oversized source locator value should fail validation");
        assert_eq!(
            oversized.issues(),
            &[CanonicalValidationIssue::SourceLocatorValueBytesExceeded {
                provenance_node: 0,
                actual: 4,
                maximum: 3,
            }]
        );

        for value in [
            ExtensionValueSchema::Bool,
            ExtensionValueSchema::String { max_bytes: 4 },
        ] {
            let cardinality = if matches!(value, ExtensionValueSchema::Bool) {
                ExtensionCardinality::ExactlyOne
            } else {
                ExtensionCardinality::OptionalOne
            };
            assert_eq!(
                ExtensionSchema::new_with_behavior(
                    schema_id.clone(),
                    ExtensionSchemaOwner::SourceIdentity,
                    value,
                    ExtensionSchemaBehavior::Canonical(ExtensionBehaviorKind::SourceObjectLocator,),
                    cardinality,
                    ExtensionReferenceRule::None,
                    ExtensionRemapRule::Preserve,
                    ExtensionInvalidationRule::Never,
                ),
                Err(ExtensionSchemaError::SourceObjectLocatorContractMismatch)
            );
        }
    }

    #[test]
    fn canonical_footprint_includes_owner_local_node_payload() {
        let draft = draft_with_children(&[&[]]);
        let outer_node_bytes = draft
            .data
            .nodes
            .capacity()
            .checked_mul(std::mem::size_of::<Node>())
            .unwrap();
        let node_allocation_bytes = draft.data.nodes[0].retained_allocation_bytes().unwrap();
        let provenance_bytes = draft
            .data
            .provenance_arena
            .as_ref()
            .retained_footprint()
            .unwrap()
            .total_bytes();
        let provenance_allocation_bytes = draft
            .data
            .provenance_arena
            .checked_allocation_footprint()
            .unwrap();
        let scene_payload_bytes = draft
            .data
            .provenance
            .checked_allocation_footprint()
            .unwrap()
            .checked_add(
                draft
                    .data
                    .extension_schemas
                    .checked_allocation_footprint()
                    .unwrap(),
            )
            .and_then(|bytes| {
                bytes.checked_add(draft.data.metadata.checked_allocation_footprint().unwrap())
            })
            .unwrap();
        let exact = u64::try_from(
            outer_node_bytes
                .checked_add(node_allocation_bytes)
                .and_then(|bytes| bytes.checked_add(provenance_bytes))
                .and_then(|bytes| bytes.checked_add(provenance_allocation_bytes))
                .and_then(|bytes| bytes.checked_add(scene_payload_bytes))
                .unwrap(),
        )
        .unwrap();

        let scene = validate_canonical_scene(
            draft,
            SceneValidationLimits::default().with_max_canonical_bytes(exact),
        )
        .expect("the exact retained footprint boundary should be inclusive");
        assert_eq!(
            scene
                .validation_stats()
                .footprint()
                .accounted_payload_bytes(),
            exact
        );

        let error = validate_canonical_scene(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default().with_max_canonical_bytes(exact - 1),
        )
        .err()
        .expect("one byte below the retained footprint must fail validation");
        assert!(
            issue_set(&error).contains(&CanonicalValidationIssue::LimitExceeded {
                limit: CanonicalValidationLimit::CanonicalBytes,
                actual: exact,
                maximum: exact - 1,
            })
        );
    }

    #[test]
    fn owned_logical_mesh_slots_use_their_real_type_size() {
        fn draft_with_logical_mesh() -> (CanonicalSceneDraft, usize) {
            let mut draft = draft_with_children(&[&[]]);
            let provenance = draft
                .data
                .provenance
                .as_ref()
                .iter()
                .next()
                .expect("fixture scene provenance");
            let mesh = mesh_with_influences_and_two_morphs(provenance);
            let nested_bytes = mesh.retained_footprint().unwrap().bytes();
            draft.data.logical_meshes.push(mesh);
            let outer_bytes = draft
                .data
                .logical_meshes
                .capacity()
                .checked_mul(std::mem::size_of::<LogicalMesh>())
                .unwrap();
            (draft, outer_bytes.checked_add(nested_bytes).unwrap())
        }

        let baseline = validate_canonical_scene(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default(),
        )
        .unwrap()
        .validation_stats()
        .footprint()
        .accounted_payload_bytes();
        let (draft, logical_mesh_bytes) = draft_with_logical_mesh();
        let exact = baseline
            .checked_add(u64::try_from(logical_mesh_bytes).unwrap())
            .unwrap();

        let scene = validate_canonical_scene(
            draft,
            SceneValidationLimits::default().with_max_canonical_bytes(exact),
        )
        .expect("the exact owned logical-mesh footprint should be admitted");
        assert_eq!(
            scene
                .validation_stats()
                .footprint()
                .accounted_payload_bytes(),
            exact
        );

        let error = validate_canonical_scene(
            draft_with_logical_mesh().0,
            SceneValidationLimits::default().with_max_canonical_bytes(exact - 1),
        )
        .err()
        .expect("one byte below the owned logical-mesh footprint must fail");
        assert!(
            issue_set(&error).contains(&CanonicalValidationIssue::LimitExceeded {
                limit: CanonicalValidationLimit::CanonicalBytes,
                actual: exact,
                maximum: exact - 1,
            })
        );
    }

    #[test]
    fn namespaced_behavior_requires_the_exact_schema_owner_and_usage() {
        let schema_id = ExtensionSchemaId::new("test:shading", 1).unwrap();
        let missing = validate_canonical_scene(
            draft_with_namespaced_shading_schema(
                schema_id.clone(),
                ExtensionSchemaSnapshot::empty(),
            ),
            SceneValidationLimits::default(),
        )
        .err()
        .expect("a missing behavior schema must fail validation");
        assert!(
            issue_set(&missing).contains(&CanonicalValidationIssue::ExtensionSchemaMissing {
                owner: CanonicalOwner::Entity {
                    kind: SceneEntityKind::Material,
                    index: 0,
                },
                behavior: ExtensionBehaviorKind::MaterialShadingModel,
                schema_id: schema_id.clone(),
            })
        );

        let wrong_owner_schema = ExtensionSchema::new(
            schema_id.clone(),
            ExtensionSchemaOwner::TextureBinding,
            ExtensionValueSchema::Bool,
            ExtensionCardinality::ExactlyOne,
            ExtensionReferenceRule::None,
            ExtensionRemapRule::Preserve,
            ExtensionInvalidationRule::OnOwnerChange,
        )
        .unwrap();
        let wrong_owner = validate_canonical_scene(
            draft_with_namespaced_shading_schema(
                schema_id.clone(),
                ExtensionSchemaSnapshot::try_from_schemas([wrong_owner_schema]).unwrap(),
            ),
            SceneValidationLimits::default(),
        )
        .err()
        .expect("an owner-compatible lookup is required");
        assert!(issue_set(&wrong_owner).contains(
            &CanonicalValidationIssue::ExtensionSchemaOwnerMismatch {
                owner: CanonicalOwner::Entity {
                    kind: SceneEntityKind::Material,
                    index: 0,
                },
                behavior: ExtensionBehaviorKind::MaterialShadingModel,
                expected: ExtensionSchemaOwner::Entity(SceneEntityKind::Material),
                actual: ExtensionSchemaOwner::TextureBinding,
            }
        ));

        let wrong_usage_schema = behavior_schema(
            schema_id,
            ExtensionSchemaOwner::Entity(SceneEntityKind::Material),
            ExtensionSchemaBehavior::Canonical(ExtensionBehaviorKind::MaterialAlphaMode),
        );
        let wrong_usage = validate_canonical_scene(
            draft_with_namespaced_shading_schema(
                ExtensionSchemaId::new("test:shading", 1).unwrap(),
                ExtensionSchemaSnapshot::try_from_schemas([wrong_usage_schema]).unwrap(),
            ),
            SceneValidationLimits::default(),
        )
        .err()
        .expect("one material behavior cannot authorize another");
        assert!(issue_set(&wrong_usage).contains(
            &CanonicalValidationIssue::ExtensionSchemaBehaviorMismatch {
                owner: CanonicalOwner::Entity {
                    kind: SceneEntityKind::Material,
                    index: 0,
                },
                expected: ExtensionSchemaBehavior::Canonical(
                    ExtensionBehaviorKind::MaterialShadingModel,
                ),
                actual: ExtensionSchemaBehavior::Canonical(
                    ExtensionBehaviorKind::MaterialAlphaMode,
                ),
            }
        ));
    }

    #[test]
    fn packed_attribute_schema_body_is_revalidated_at_publication() {
        let (arena, provenance) = test_provenance_fixture();
        let schema_id = ExtensionSchemaId::new("test:packed-normal", 1).unwrap();
        let source_snapshot = ExtensionSchemaSnapshot::try_from_schemas([packed_schema(
            schema_id.clone(),
            PackedByteOrder::LittleEndian,
        )])
        .unwrap();
        let encoding =
            RegisteredPackedEncoding::try_from_schema(&source_snapshot, schema_id.clone()).unwrap();
        let format =
            AttributeFormat::new(ComponentType::F32, 3, Normalization::None, Some(encoding))
                .unwrap();
        let attribute = Attribute::new(
            AttributeSemantic::Custom(CustomAttributeSemantic::from_name(schema_id.name().clone())),
            AttributeDomain::Vertex,
            format.clone(),
            AttributePayload::dense(AttributeBuffer::from_packed(format, vec![0; 12]).unwrap()),
            provenance,
        )
        .unwrap();
        let primitive = MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            vec![positions(3, provenance), attribute],
            None,
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        )
        .unwrap();
        let mismatched_snapshot = ExtensionSchemaSnapshot::try_from_schemas([packed_schema(
            schema_id,
            PackedByteOrder::BigEndian,
        )])
        .unwrap();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            mismatched_snapshot,
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        builder
            .try_push_logical_mesh(
                LogicalMesh::try_new(
                    None,
                    vec![primitive],
                    Vec::new(),
                    Vec::new(),
                    LogicalMeshBounds::default(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();

        let error =
            validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                .err()
                .expect("packed schema drift must fail central validation");
        assert!(issue_set(&error).contains(
            &CanonicalValidationIssue::PackedAttributeSchemaMismatch {
                owner: CanonicalOwner::Attribute {
                    owner: CanonicalAttributeOwner::Primitive {
                        logical_mesh: 0,
                        primitive: 0,
                    },
                    attribute: 1,
                },
                reason: RegisteredPackedEncodingError::SchemaBodyMismatch,
            }
        ));
    }

    #[test]
    fn node_limit_accepts_exact_boundary_and_rejects_one_less() {
        validate_canonical_scene(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default().with_max_nodes(1),
        )
        .expect("exact node limit should validate");

        let error = validate_canonical_scene(
            draft_with_children(&[&[]]),
            SceneValidationLimits::default().with_max_nodes(0),
        )
        .err()
        .expect("a node limit below the scene size must fail validation");
        assert!(
            issue_set(&error).contains(&CanonicalValidationIssue::LimitExceeded {
                limit: CanonicalValidationLimit::Nodes,
                actual: 1,
                maximum: 0,
            })
        );
    }

    #[test]
    fn payload_cell_limit_counts_attribute_storage_at_the_exact_boundary() {
        let build_draft = || {
            let (arena, provenance) = test_provenance_fixture();
            let primitive = MeshPrimitive::try_new(
                3,
                Topology::TriangleList,
                IndexStream::non_indexed(),
                None,
                vec![positions(3, provenance)],
                None,
                Vec::new(),
                0,
                PrimitiveBounds::default(),
                provenance,
            )
            .unwrap();
            let mut builder = CanonicalSceneBuilder::try_new(
                SceneSpace::unknown(),
                ExtensionSchemaSnapshot::empty(),
                arena,
                provenance,
                DerivedAbsenceReason::NotProvided,
            )
            .unwrap();
            let root = builder
                .try_push_node_unchecked_for_test(node(provenance, []))
                .unwrap();
            builder.set_root_unchecked_for_test(root);
            builder
                .try_push_logical_mesh(
                    LogicalMesh::try_new(
                        None,
                        vec![primitive],
                        Vec::new(),
                        Vec::new(),
                        LogicalMeshBounds::default(),
                        provenance,
                    )
                    .unwrap(),
                )
                .unwrap();
            builder.finish_draft()
        };

        let scene = validate_canonical_scene(
            build_draft(),
            SceneValidationLimits::default().with_max_payload_cells(3),
        )
        .expect("three stored position elements must fit the exact cell boundary");
        assert_eq!(scene.validation_stats().canonical_counts().payload_cells, 3);

        let error = validate_canonical_scene(
            build_draft(),
            SceneValidationLimits::default().with_max_payload_cells(2),
        )
        .err()
        .expect("one cell below the stored position payload must fail");
        assert!(
            issue_set(&error).contains(&CanonicalValidationIssue::LimitExceeded {
                limit: CanonicalValidationLimit::PayloadCells,
                actual: 3,
                maximum: 2,
            })
        );
    }

    #[test]
    fn metadata_limits_cover_aggregate_and_per_entry_boundaries() {
        let key = "test:key";
        let value = "value";
        let build_draft = || {
            let (arena, provenance) = test_provenance_fixture();
            let metadata = crate::canonical_scene::SceneMetadata::try_new([ExtensionEntry::new(
                NamespacedName::new(key).unwrap(),
                ExtensionValue::try_string(value).unwrap(),
                provenance,
                ExtensionEntryClassification::DescriptiveMetadata,
            )])
            .unwrap();
            let mut builder = CanonicalSceneBuilder::try_new(
                SceneSpace::unknown(),
                ExtensionSchemaSnapshot::empty(),
                arena,
                provenance,
                DerivedAbsenceReason::NotProvided,
            )
            .unwrap();
            let root = builder
                .try_push_node_unchecked_for_test(node(provenance, []))
                .unwrap();
            builder.set_root_unchecked_for_test(root);
            builder.set_metadata(metadata).unwrap();
            builder.finish_draft()
        };
        let aggregate_bytes = u64::try_from(key.len() + value.len()).unwrap();

        validate_canonical_scene(
            build_draft(),
            SceneValidationLimits::default()
                .with_max_metadata_entries(1)
                .with_max_metadata_bytes(aggregate_bytes)
                .with_max_metadata_key_bytes(key.len())
                .with_max_metadata_string_bytes(value.len()),
        )
        .expect("all exact metadata boundaries must be inclusive");

        let error = validate_canonical_scene(
            build_draft(),
            SceneValidationLimits::default()
                .with_max_metadata_entries(0)
                .with_max_metadata_bytes(aggregate_bytes - 1)
                .with_max_metadata_key_bytes(key.len() - 1)
                .with_max_metadata_string_bytes(value.len() - 1),
        )
        .err()
        .expect("every one-less metadata boundary must fail");
        let issues = issue_set(&error);
        assert!(
            issues.contains(&CanonicalValidationIssue::MetadataKeyBytesExceeded {
                owner: CanonicalExtensionOwner::SceneMetadata,
                entry: 0,
                actual: key.len(),
                maximum: key.len() - 1,
            })
        );
        assert!(
            issues.contains(&CanonicalValidationIssue::MetadataStringBytesExceeded {
                owner: CanonicalExtensionOwner::SceneMetadata,
                entry: 0,
                actual: value.len(),
                maximum: value.len() - 1,
            })
        );
        assert!(issues.contains(&CanonicalValidationIssue::LimitExceeded {
            limit: CanonicalValidationLimit::MetadataEntries,
            actual: 1,
            maximum: 0,
        }));
        assert!(issues.contains(&CanonicalValidationIssue::LimitExceeded {
            limit: CanonicalValidationLimit::MetadataBytes,
            actual: aggregate_bytes,
            maximum: aggregate_bytes - 1,
        }));
    }

    #[test]
    fn every_direct_scene_reference_is_validated_with_its_owner_path() {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(
                Node::try_new_unchecked_references_for_test(
                    None,
                    AuthoredTransform::identity_trs(),
                    [],
                    Some(CameraIndex::from_u32(11)),
                    Some(LightIndex::from_u32(12)),
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        builder
            .try_push_texture_unchecked_for_test(Texture::from_unchecked_references_for_test(
                ImageIndex::from_u32(13),
                SamplerIndex::from_u32(14),
                provenance,
            ))
            .unwrap();
        builder
            .try_push_mesh_instance_unchecked_for_test(
                MeshInstance::try_new_unchecked_references_for_test(
                    NodeIndex::from_u32(15),
                    LogicalMeshIndex::from_u32(16),
                    Some(SkinIndex::from_u32(17)),
                    None,
                    Vec::new(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        builder
            .try_push_skin_unchecked_for_test(
                Skin::try_new_unchecked_references_for_test(
                    None,
                    [NodeIndex::from_u32(18)],
                    [Mat4::IDENTITY],
                    Some(NodeIndex::from_u32(19)),
                    SkinMatrixSpaces::canonical(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();

        let error =
            validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                .err()
                .expect("out-of-range scene references must fail validation");
        let expected = [
            (
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::Node,
                    index: 0,
                },
                SceneEntityKind::Camera,
                11,
            ),
            (
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::Node,
                    index: 0,
                },
                SceneEntityKind::Light,
                12,
            ),
            (
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::Texture,
                    index: 0,
                },
                SceneEntityKind::Image,
                13,
            ),
            (
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::Texture,
                    index: 0,
                },
                SceneEntityKind::Sampler,
                14,
            ),
            (
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::MeshInstance,
                    index: 0,
                },
                SceneEntityKind::Node,
                15,
            ),
            (
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::MeshInstance,
                    index: 0,
                },
                SceneEntityKind::LogicalMesh,
                16,
            ),
            (
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::MeshInstance,
                    index: 0,
                },
                SceneEntityKind::Skin,
                17,
            ),
            (
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::Skin,
                    index: 0,
                },
                SceneEntityKind::Node,
                18,
            ),
            (
                CanonicalOwner::Entity {
                    kind: SceneEntityKind::Skin,
                    index: 0,
                },
                SceneEntityKind::Node,
                19,
            ),
        ];
        for (owner, target, target_index) in expected {
            assert!(
                issue_set(&error).contains(&CanonicalValidationIssue::ReferenceOutOfRange {
                    owner,
                    target,
                    target_index,
                })
            );
        }
    }

    #[test]
    fn nested_primitive_and_binding_references_retain_owner_paths() {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        builder
            .try_push_logical_mesh(
                LogicalMesh::try_new(
                    None,
                    vec![primitive_with_material(
                        MaterialIndex::from_u32(21),
                        provenance,
                    )],
                    Vec::new(),
                    Vec::new(),
                    LogicalMeshBounds::default(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        builder
            .try_push_material_unchecked_for_test(material_with_unchecked_texture(
                TextureIndex::from_u32(22),
                provenance,
            ))
            .unwrap();

        let error =
            validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                .err()
                .expect("nested out-of-range references must fail validation");
        assert!(
            issue_set(&error).contains(&CanonicalValidationIssue::ReferenceOutOfRange {
                owner: CanonicalOwner::Primitive {
                    logical_mesh: 0,
                    primitive: 0,
                },
                target: SceneEntityKind::Material,
                target_index: 21,
            })
        );
        assert!(
            issue_set(&error).contains(&CanonicalValidationIssue::ReferenceOutOfRange {
                owner: CanonicalOwner::TextureBinding {
                    material: 0,
                    binding: 0,
                },
                target: SceneEntityKind::Texture,
                target_index: 22,
            })
        );
    }

    #[test]
    fn animation_targets_resolve_and_trs_channels_reject_matrix_authored_nodes() {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(
                Node::try_new_unchecked_references_for_test(
                    None,
                    AuthoredTransform::try_matrix(Mat4::IDENTITY).unwrap(),
                    [],
                    None,
                    None,
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        builder
            .try_push_animation_clip(
                AnimationClip::try_new(
                    0.0,
                    AnimationTimeDomain::Seconds,
                    [
                        translation_channel(root, provenance),
                        scale_channel(NodeIndex::from_u32(23), provenance),
                    ],
                    provenance,
                    &AnimationLimits::canonical(),
                )
                .unwrap(),
            )
            .unwrap();

        let error =
            validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                .err()
                .expect("invalid animation target semantics must fail validation");
        assert!(issue_set(&error).contains(
            &CanonicalValidationIssue::MatrixAuthoredNodeTrsChannel {
                owner: CanonicalOwner::AnimationChannel {
                    clip: 0,
                    channel: 0,
                },
                node: 0,
            },
        ));
        assert!(
            issue_set(&error).contains(&CanonicalValidationIssue::ReferenceOutOfRange {
                owner: CanonicalOwner::AnimationChannel {
                    clip: 0,
                    channel: 1,
                },
                target: SceneEntityKind::Node,
                target_index: 23,
            })
        );
    }

    #[derive(Clone, Copy)]
    enum DerivedBoundsFault {
        PrimitiveGeneration,
        PrimitiveStamp,
        LogicalMeshGeneration,
        LogicalMeshStamp,
        SceneGeneration,
        SceneStamp,
    }

    fn bounds_producer(code: &str, operation: bool) -> DerivedProducer {
        if !operation {
            return DerivedProducer::source();
        }
        DerivedProducer::operation(
            OperationIdentity::new(
                OperationKind::Generated,
                OptionsDigest::sha256([0x5a; 32]),
                [
                    OperationEvidenceSummary::new(NamespacedName::new(code).unwrap(), 1, None)
                        .unwrap(),
                ],
            )
            .unwrap(),
        )
    }

    fn draft_with_derived_bounds_fault(
        fault: Option<DerivedBoundsFault>,
        operation_producers: bool,
    ) -> CanonicalSceneDraft {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let generation = builder.generation();
        let foreign_generation = crate::identity::SceneGenerationId::allocate().unwrap();
        let forged_stamp = InputStamp::from_bytes([0xa5; 32]);

        let primitive_stamp = primitive_bounds_input_stamp(
            &bounds_fixture_primitive(
                provenance,
                bounds_fixture_positions(),
                Topology::TriangleList,
                PrimitiveBounds::absent(
                    DerivedAbsenceReason::NotProvided,
                    MorphBoundsPolicy::BaseGeometry,
                ),
            ),
            &[],
        )
        .unwrap()
        .stamp();
        let build_primitive = || {
            let primitive_generation =
                if matches!(fault, Some(DerivedBoundsFault::PrimitiveGeneration)) {
                    foreign_generation
                } else {
                    generation
                };
            let primitive_stamp = if matches!(fault, Some(DerivedBoundsFault::PrimitiveStamp)) {
                forged_stamp
            } else {
                primitive_stamp
            };
            bounds_fixture_primitive(
                provenance,
                bounds_fixture_positions(),
                Topology::TriangleList,
                PrimitiveBounds::try_from_validated(
                    Derived::valid(
                        BOUNDS_FIXTURE_AABB,
                        primitive_generation,
                        primitive_stamp,
                        bounds_producer("test:primitive-bounds", operation_producers),
                    ),
                    MorphBoundsPolicy::BaseGeometry,
                )
                .unwrap(),
            )
        };

        let provisional_mesh = LogicalMesh::try_new(
            None,
            vec![build_primitive()],
            Vec::new(),
            Vec::new(),
            LogicalMeshBounds::absent(
                DerivedAbsenceReason::NotProvided,
                MorphBoundsPolicy::BaseGeometry,
            ),
            provenance,
        )
        .unwrap();
        let logical_mesh_stamp = logical_mesh_bounds_input_stamp(&provisional_mesh)
            .unwrap()
            .stamp();
        let logical_mesh_generation =
            if matches!(fault, Some(DerivedBoundsFault::LogicalMeshGeneration)) {
                foreign_generation
            } else {
                generation
            };
        let logical_mesh_stamp = if matches!(fault, Some(DerivedBoundsFault::LogicalMeshStamp)) {
            forged_stamp
        } else {
            logical_mesh_stamp
        };
        let logical_mesh = LogicalMesh::try_new(
            None,
            vec![build_primitive()],
            Vec::new(),
            Vec::new(),
            LogicalMeshBounds::try_from_validated(
                Derived::valid(
                    BOUNDS_FIXTURE_AABB,
                    logical_mesh_generation,
                    logical_mesh_stamp,
                    bounds_producer("test:logical-mesh-bounds", operation_producers),
                ),
                MorphBoundsPolicy::BaseGeometry,
            )
            .unwrap(),
            provenance,
        )
        .unwrap();

        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        let root_id = root.handle(builder.generation());
        let logical_mesh = builder.try_push_logical_mesh(logical_mesh).unwrap();
        builder
            .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
                root_id,
                logical_mesh,
                None,
                None,
                Vec::new(),
                provenance,
            ))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        let mut draft = builder.finish_draft();
        let scene_stamp = scene_bounds_input_stamp(&draft.data).unwrap().stamp();
        let scene_generation = if matches!(fault, Some(DerivedBoundsFault::SceneGeneration)) {
            foreign_generation
        } else {
            generation
        };
        let scene_stamp = if matches!(fault, Some(DerivedBoundsFault::SceneStamp)) {
            forged_stamp
        } else {
            scene_stamp
        };
        draft.data.scene_bounds = SceneBounds::try_from_validated(Derived::valid(
            BOUNDS_FIXTURE_AABB,
            scene_generation,
            scene_stamp,
            bounds_producer("test:scene-bounds", operation_producers),
        ))
        .unwrap();
        draft
    }

    fn scene_stamp_fixture(translation_x: f32, logical_mesh_reference: u32) -> InputStamp {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(
                Node::try_new_unchecked_references_for_test(
                    None,
                    AuthoredTransform::try_trs(
                        Vec3::new(translation_x, 0.0, 0.0),
                        Quat::IDENTITY,
                        Vec3::new(1.0, 1.0, 1.0),
                    )
                    .unwrap(),
                    [],
                    None,
                    None,
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        builder
            .try_push_logical_mesh(
                LogicalMesh::try_new(
                    None,
                    vec![bounds_fixture_primitive(
                        provenance,
                        bounds_fixture_positions(),
                        Topology::TriangleList,
                        PrimitiveBounds::default(),
                    )],
                    Vec::new(),
                    Vec::new(),
                    LogicalMeshBounds::default(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        builder
            .try_push_mesh_instance_unchecked_for_test(
                MeshInstance::try_new_unchecked_references_for_test(
                    root,
                    LogicalMeshIndex::from_u32(logical_mesh_reference),
                    None,
                    None,
                    Vec::new(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        scene_bounds_input_stamp(&builder.finish_draft().data)
            .unwrap()
            .stamp()
    }

    #[test]
    fn influences_without_a_selected_skin_remain_inert_source_data() {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        let root_id = root.handle(builder.generation());
        let mesh = builder
            .try_push_logical_mesh(mesh_with_influences_and_two_morphs(provenance))
            .unwrap();
        builder
            .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
                root_id,
                mesh,
                None,
                None,
                Vec::new(),
                provenance,
            ))
            .unwrap();

        validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
            .expect("unbound influence rows are preservable source data");
    }

    #[test]
    fn shared_mesh_palette_validation_is_linear_in_instances_plus_influences() {
        let validation_work = |influence_count, instance_count| {
            validate_canonical_scene(
                draft_with_shared_skinned_instances(influence_count, instance_count),
                SceneValidationLimits::default(),
            )
            .expect("shared skinned mesh fixture should validate")
            .validation_stats()
            .work_units()
        };

        let small_instance_delta = validation_work(1, 10) - validation_work(1, 1);
        let large_instance_delta = validation_work(100, 10) - validation_work(100, 1);
        assert_eq!(
            large_instance_delta, small_instance_delta,
            "adding instances must not rescan a shared mesh's influence cells"
        );

        let single_influence = validation_work(1, 1);
        let two_influences = validation_work(2, 1);
        let hundred_influences = validation_work(100, 1);
        let per_influence = two_influences - single_influence;
        assert_eq!(
            hundred_influences - single_influence,
            99 * per_influence,
            "influence traversal must add one stable linear slope"
        );
    }

    #[test]
    fn selected_skin_and_morph_widths_are_revalidated_against_real_owners() {
        let (arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();
        let root = builder
            .try_push_node_unchecked_for_test(node(provenance, []))
            .unwrap();
        builder.set_root_unchecked_for_test(root);
        let root_id = root.handle(builder.generation());
        let mesh = builder
            .try_push_logical_mesh(mesh_with_influences_and_two_morphs(provenance))
            .unwrap();
        let skin_id = builder
            .try_push_skin(
                CanonicalSkinDescriptor::try_new(
                    None,
                    [root_id],
                    [Mat4::IDENTITY],
                    None,
                    SkinMatrixSpaces::canonical(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        let mesh = LogicalMeshIndex::from_handle(builder.generation(), mesh).unwrap();
        let skin = SkinIndex::from_handle(builder.generation(), skin_id).unwrap();
        let instance = builder
            .try_push_mesh_instance_unchecked_for_test(
                MeshInstance::try_new_unchecked_references_for_test(
                    root,
                    mesh,
                    Some(skin),
                    Some(vec![0.5]),
                    Vec::new(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        let instance = MeshInstanceIndex::from_handle(builder.generation(), instance).unwrap();
        builder
            .try_push_animation_clip(
                AnimationClip::try_new(
                    0.0,
                    AnimationTimeDomain::Seconds,
                    [morph_channel(instance, provenance)],
                    provenance,
                    &AnimationLimits::canonical(),
                )
                .unwrap(),
            )
            .unwrap();

        let error =
            validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
                .err()
                .expect("selected skin and morph widths require owner validation");
        assert!(issue_set(&error).contains(
            &CanonicalValidationIssue::SkinPaletteJointOutOfRange {
                instance: 0,
                logical_mesh: 0,
                primitive: 0,
                influence: 0,
                palette_joint: 1,
                palette_len: 1,
            },
        ));
        assert!(
            issue_set(&error).contains(&CanonicalValidationIssue::MorphWeightWidthMismatch {
                owner: CanonicalOwner::Entity {
                    kind: SceneEntityKind::MeshInstance,
                    index: 0,
                },
                expected: 2,
                actual: 1,
            },)
        );
        assert!(
            issue_set(&error).contains(&CanonicalValidationIssue::MorphWeightWidthMismatch {
                owner: CanonicalOwner::AnimationChannel {
                    clip: 0,
                    channel: 0,
                },
                expected: 2,
                actual: 1,
            },)
        );
    }

    #[test]
    fn derived_bound_stamps_are_domain_separated_deterministic_and_dependency_sensitive() {
        let (_arena, provenance) = test_provenance_fixture();
        let primitive = |values, topology, policy| {
            bounds_fixture_primitive(
                provenance,
                values,
                topology,
                PrimitiveBounds::absent(DerivedAbsenceReason::NotProvided, policy),
            )
        };
        let baseline = primitive_bounds_input_stamp(
            &primitive(
                bounds_fixture_positions(),
                Topology::TriangleList,
                MorphBoundsPolicy::BaseGeometry,
            ),
            &[],
        )
        .unwrap()
        .stamp();
        assert_eq!(
            baseline,
            primitive_bounds_input_stamp(
                &primitive(
                    bounds_fixture_positions(),
                    Topology::TriangleList,
                    MorphBoundsPolicy::BaseGeometry,
                ),
                &[]
            )
            .unwrap()
            .stamp()
        );
        let mut changed_positions = bounds_fixture_positions();
        changed_positions[2].y = 2.0;
        assert_ne!(
            baseline,
            primitive_bounds_input_stamp(
                &primitive(
                    changed_positions,
                    Topology::TriangleList,
                    MorphBoundsPolicy::BaseGeometry,
                ),
                &[]
            )
            .unwrap()
            .stamp()
        );
        assert_ne!(
            baseline,
            primitive_bounds_input_stamp(
                &primitive(
                    bounds_fixture_positions(),
                    Topology::TriangleStrip,
                    MorphBoundsPolicy::BaseGeometry,
                ),
                &[]
            )
            .unwrap()
            .stamp()
        );
        assert_ne!(
            baseline,
            primitive_bounds_input_stamp(
                &primitive(
                    bounds_fixture_positions(),
                    Topology::TriangleList,
                    MorphBoundsPolicy::DefaultWeights,
                ),
                &[]
            )
            .unwrap()
            .stamp()
        );

        let logical_mesh = LogicalMesh::try_new(
            None,
            vec![primitive(
                bounds_fixture_positions(),
                Topology::TriangleList,
                MorphBoundsPolicy::BaseGeometry,
            )],
            Vec::new(),
            Vec::new(),
            LogicalMeshBounds::default(),
            provenance,
        )
        .unwrap();
        let logical_stamp = logical_mesh_bounds_input_stamp(&logical_mesh)
            .unwrap()
            .stamp();
        assert_ne!(baseline, logical_stamp);

        let scene_stamp = scene_stamp_fixture(0.0, 0);
        assert_eq!(scene_stamp, scene_stamp_fixture(0.0, 0));
        assert_ne!(scene_stamp, scene_stamp_fixture(1.0, 0));
        assert_ne!(scene_stamp, scene_stamp_fixture(0.0, 1));
        assert_ne!(logical_stamp, scene_stamp);
    }

    #[test]
    fn morph_bounds_stamps_close_over_policy_specific_deltas_and_default_weights() {
        let (_arena, provenance) = test_provenance_fixture();
        let primitive_default =
            bounds_fixture_primitive_with_morph(provenance, 1.0, MorphBoundsPolicy::DefaultWeights);
        assert_ne!(
            primitive_bounds_input_stamp(&primitive_default, &[0.25])
                .unwrap()
                .stamp(),
            primitive_bounds_input_stamp(&primitive_default, &[0.75])
                .unwrap()
                .stamp(),
            "primitive DefaultWeights bounds must depend on owning mesh defaults"
        );

        let logical_stamp = |primitive_policy, logical_policy, delta_y, weight| {
            let mesh = LogicalMesh::try_new(
                None,
                vec![bounds_fixture_primitive_with_morph(
                    provenance,
                    delta_y,
                    primitive_policy,
                )],
                vec![MorphTargetDefinition::try_new(None, provenance).unwrap()],
                vec![weight],
                LogicalMeshBounds::absent(DerivedAbsenceReason::NotProvided, logical_policy),
                provenance,
            )
            .unwrap();
            logical_mesh_bounds_input_stamp(&mesh).unwrap().stamp()
        };

        for primitive_policy in [
            MorphBoundsPolicy::BaseGeometry,
            MorphBoundsPolicy::DefaultWeights,
        ] {
            let baseline = logical_stamp(
                primitive_policy,
                MorphBoundsPolicy::DefaultWeights,
                1.0,
                0.25,
            );
            assert_ne!(
                baseline,
                logical_stamp(
                    primitive_policy,
                    MorphBoundsPolicy::DefaultWeights,
                    2.0,
                    0.25,
                ),
                "logical DefaultWeights must include position deltas for every child policy"
            );
            assert_ne!(
                baseline,
                logical_stamp(
                    primitive_policy,
                    MorphBoundsPolicy::DefaultWeights,
                    1.0,
                    0.75,
                ),
                "logical DefaultWeights must include the applied default weights"
            );
        }
    }

    #[test]
    fn scene_bounds_stamp_includes_skin_contract_and_influence_payload() {
        let baseline = draft_with_shared_skinned_instances(1, 1);
        let baseline_stamp = scene_bounds_input_stamp(&baseline.data).unwrap().stamp();
        let changed_influences = draft_with_shared_skinned_instances(2, 1);
        assert_ne!(
            baseline_stamp,
            scene_bounds_input_stamp(&changed_influences.data)
                .unwrap()
                .stamp()
        );

        let mut changed_skin = draft_with_shared_skinned_instances(1, 1);
        let provenance = changed_skin.data.skins[0].as_ref().provenance();
        changed_skin.data.skins[0] = SharedStorage::try_new(
            Skin::try_new_unchecked_references_for_test(
                None,
                [NodeIndex::from_u32(0)],
                [Mat4::IDENTITY],
                Some(NodeIndex::from_u32(0)),
                SkinMatrixSpaces::canonical(),
                provenance,
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(
            baseline_stamp,
            scene_bounds_input_stamp(&changed_skin.data)
                .unwrap()
                .stamp()
        );
    }

    #[test]
    fn large_derived_stamp_stops_at_the_controlled_work_boundary() {
        let (_arena, provenance) = test_provenance_fixture();
        let values = (0..1_000)
            .map(|index| Vec3::new(index as f32, 0.0, 0.0))
            .collect::<Vec<_>>();
        let primitive = bounds_fixture_primitive(
            provenance,
            values,
            Topology::Points,
            PrimitiveBounds::default(),
        );
        let mut charged = 0_u64;
        let error = primitive_bounds_input_stamp_controlled(&primitive, &[], &mut |amount| {
            charged += amount;
            if charged > SCENE_VALIDATION_POLL_INTERVAL {
                Err("cancelled")
            } else {
                Ok(())
            }
        })
        .err()
        .expect("large stamp encoding must propagate controlled cancellation");
        assert!(matches!(
            error,
            ControlledDerivedInputStampError::Control("cancelled")
        ));
        assert_eq!(charged, SCENE_VALIDATION_POLL_INTERVAL + 1);
    }

    #[test]
    fn central_validator_requires_generation_and_input_stamp_for_every_bounds_owner() {
        validate_canonical_scene(
            draft_with_derived_bounds_fault(None, false),
            SceneValidationLimits::default(),
        )
        .expect("complete derived bounds evidence should publish");

        let cases = [
            (
                DerivedBoundsFault::PrimitiveGeneration,
                CanonicalValidationIssue::DerivedGenerationMismatch {
                    owner: CanonicalOwner::Primitive {
                        logical_mesh: 0,
                        primitive: 0,
                    },
                },
            ),
            (
                DerivedBoundsFault::PrimitiveStamp,
                CanonicalValidationIssue::DerivedInputStampMismatch {
                    owner: CanonicalOwner::Primitive {
                        logical_mesh: 0,
                        primitive: 0,
                    },
                },
            ),
            (
                DerivedBoundsFault::LogicalMeshGeneration,
                CanonicalValidationIssue::DerivedGenerationMismatch {
                    owner: CanonicalOwner::Entity {
                        kind: SceneEntityKind::LogicalMesh,
                        index: 0,
                    },
                },
            ),
            (
                DerivedBoundsFault::LogicalMeshStamp,
                CanonicalValidationIssue::DerivedInputStampMismatch {
                    owner: CanonicalOwner::Entity {
                        kind: SceneEntityKind::LogicalMesh,
                        index: 0,
                    },
                },
            ),
            (
                DerivedBoundsFault::SceneGeneration,
                CanonicalValidationIssue::DerivedGenerationMismatch {
                    owner: CanonicalOwner::Scene,
                },
            ),
            (
                DerivedBoundsFault::SceneStamp,
                CanonicalValidationIssue::DerivedInputStampMismatch {
                    owner: CanonicalOwner::Scene,
                },
            ),
        ];
        for (fault, expected) in cases {
            let error = validate_canonical_scene(
                draft_with_derived_bounds_fault(Some(fault), false),
                SceneValidationLimits::default(),
            )
            .err()
            .expect("forged derived bounds evidence must not publish");
            assert_eq!(error.issues(), &[expected]);
        }
    }

    #[test]
    fn canonical_byte_limit_counts_every_derived_operation_producer_at_the_exact_boundary() {
        let source = validate_canonical_scene(
            draft_with_derived_bounds_fault(None, false),
            SceneValidationLimits::default(),
        )
        .unwrap();
        let source_bytes = source
            .validation_stats()
            .footprint()
            .accounted_payload_bytes();

        let operation = validate_canonical_scene(
            draft_with_derived_bounds_fault(None, true),
            SceneValidationLimits::default(),
        )
        .unwrap();
        let operation_bytes = operation
            .validation_stats()
            .footprint()
            .accounted_payload_bytes();
        let producer_bytes = u64::try_from(
            bounds_producer("test:footprint", true)
                .retained_allocation_bytes()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            operation_bytes - source_bytes,
            producer_bytes * 3,
            "primitive, logical-mesh, and scene bounds producers must all be retained"
        );

        validate_canonical_scene(
            draft_with_derived_bounds_fault(None, true),
            SceneValidationLimits::default().with_max_canonical_bytes(operation_bytes),
        )
        .expect("the exact canonical-byte boundary must be inclusive");
        let error = validate_canonical_scene(
            draft_with_derived_bounds_fault(None, true),
            SceneValidationLimits::default().with_max_canonical_bytes(operation_bytes - 1),
        )
        .err()
        .expect("one byte below the retained operation producers must fail");
        assert!(
            error
                .issues()
                .contains(&CanonicalValidationIssue::LimitExceeded {
                    limit: CanonicalValidationLimit::CanonicalBytes,
                    actual: operation_bytes,
                    maximum: operation_bytes - 1,
                })
        );
    }
}
