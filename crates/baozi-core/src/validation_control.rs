use crate::{SceneValidationError, ValidatedScene};
use std::convert::Infallible;
use std::error::Error;
use std::fmt;
use thiserror::Error;

/// Maximum work between two cooperative cancellation polls.
pub const SCENE_VALIDATION_POLL_INTERVAL: u64 = 256;

/// A stable category of work charged by the core validator or edit cloner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneValidationWork {
    GraphNode,
    GraphEdge,
    Entity,
    Reference,
    PayloadCell,
    MetadataEntry,
    MetadataByte,
    Traversal,
    CloneCell,
}

/// The lifetime classification of storage controlled by a validation adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneReservationKind {
    ValidationScratch,
    CanonicalOutput,
    EditCandidate,
}

/// Immutable limits applied by the central validator and controlled edit clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneValidationLimits {
    max_nodes: u64,
    max_edges: u64,
    max_depth: u64,
    max_entities: u64,
    max_references: u64,
    max_payload_cells: u64,
    max_work: u64,
    max_metadata_entries: u64,
    max_metadata_bytes: u64,
    max_metadata_key_bytes: usize,
    max_metadata_string_bytes: usize,
    max_canonical_bytes: u64,
    max_scratch_bytes: u64,
}

impl SceneValidationLimits {
    pub const fn with_max_nodes(mut self, value: u64) -> Self {
        self.max_nodes = value;
        self
    }

    pub const fn with_max_edges(mut self, value: u64) -> Self {
        self.max_edges = value;
        self
    }

    pub const fn with_max_depth(mut self, value: u64) -> Self {
        self.max_depth = value;
        self
    }

    pub const fn with_max_entities(mut self, value: u64) -> Self {
        self.max_entities = value;
        self
    }

    pub const fn with_max_references(mut self, value: u64) -> Self {
        self.max_references = value;
        self
    }

    pub const fn with_max_payload_cells(mut self, value: u64) -> Self {
        self.max_payload_cells = value;
        self
    }

    pub const fn with_max_work(mut self, value: u64) -> Self {
        self.max_work = value;
        self
    }

    pub const fn with_max_metadata_entries(mut self, value: u64) -> Self {
        self.max_metadata_entries = value;
        self
    }

    pub const fn with_max_metadata_bytes(mut self, value: u64) -> Self {
        self.max_metadata_bytes = value;
        self
    }

    pub const fn with_max_metadata_key_bytes(mut self, value: usize) -> Self {
        self.max_metadata_key_bytes = value;
        self
    }

    pub const fn with_max_metadata_string_bytes(mut self, value: usize) -> Self {
        self.max_metadata_string_bytes = value;
        self
    }

    pub const fn with_max_canonical_bytes(mut self, value: u64) -> Self {
        self.max_canonical_bytes = value;
        self
    }

    pub const fn with_max_scratch_bytes(mut self, value: u64) -> Self {
        self.max_scratch_bytes = value;
        self
    }

    pub const fn max_nodes(self) -> u64 {
        self.max_nodes
    }

    pub const fn max_edges(self) -> u64 {
        self.max_edges
    }

    pub const fn max_depth(self) -> u64 {
        self.max_depth
    }

    pub const fn max_entities(self) -> u64 {
        self.max_entities
    }

    pub const fn max_references(self) -> u64 {
        self.max_references
    }

    pub const fn max_payload_cells(self) -> u64 {
        self.max_payload_cells
    }

    pub const fn max_work(self) -> u64 {
        self.max_work
    }

    pub const fn max_metadata_entries(self) -> u64 {
        self.max_metadata_entries
    }

    pub const fn max_metadata_bytes(self) -> u64 {
        self.max_metadata_bytes
    }

    pub const fn max_metadata_key_bytes(self) -> usize {
        self.max_metadata_key_bytes
    }

    pub const fn max_metadata_string_bytes(self) -> usize {
        self.max_metadata_string_bytes
    }

    pub const fn max_canonical_bytes(self) -> u64 {
        self.max_canonical_bytes
    }

    pub const fn max_scratch_bytes(self) -> u64 {
        self.max_scratch_bytes
    }
}

impl Default for SceneValidationLimits {
    fn default() -> Self {
        Self {
            max_nodes: 1_000_000,
            max_edges: 4_000_000,
            max_depth: 100_000,
            max_entities: 2_000_000,
            max_references: 8_000_000,
            max_payload_cells: 100_000_000,
            max_work: 1_000_000_000,
            max_metadata_entries: 1_000_000,
            max_metadata_bytes: 64 * 1024 * 1024,
            max_metadata_key_bytes: 256,
            max_metadata_string_bytes: 1024 * 1024,
            max_canonical_bytes: 4 * 1024 * 1024 * 1024,
            max_scratch_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Canonical values measured once, independently from validator work passes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneCanonicalCounts {
    pub nodes: u64,
    pub edges: u64,
    pub entities: u64,
    pub references: u64,
    pub payload_cells: u64,
    pub metadata_entries: u64,
    pub metadata_bytes: u64,
    pub embedded_bytes: u64,
}

/// Deterministic Baozi payload bytes visible through stable container APIs.
///
/// This includes retained `Vec`, `String`, and Baozi-owned `OrderedMap` backing
/// capacity. Allocator-private metadata and size-class rounding remain separate
/// RSS evidence rather than canonical payload bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneFootprint {
    accounted_payload_bytes: u64,
}

impl SceneFootprint {
    pub(crate) const fn new(accounted_payload_bytes: u64) -> Self {
        Self {
            accounted_payload_bytes,
        }
    }

    pub const fn accounted_payload_bytes(self) -> u64 {
        self.accounted_payload_bytes
    }
}

/// Deterministic evidence produced by one complete central validation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneValidationStats {
    canonical_counts: SceneCanonicalCounts,
    footprint: SceneFootprint,
    work_units: u64,
    maximum_depth: u64,
    scratch_peak_bytes: u64,
}

impl SceneValidationStats {
    pub(crate) const fn new(
        canonical_counts: SceneCanonicalCounts,
        footprint: SceneFootprint,
        work_units: u64,
        maximum_depth: u64,
        scratch_peak_bytes: u64,
    ) -> Self {
        Self {
            canonical_counts,
            footprint,
            work_units,
            maximum_depth,
            scratch_peak_bytes,
        }
    }

    pub const fn canonical_counts(self) -> SceneCanonicalCounts {
        self.canonical_counts
    }

    pub const fn footprint(self) -> SceneFootprint {
        self.footprint
    }

    pub const fn work_units(self) -> u64 {
        self.work_units
    }

    pub const fn maximum_depth(self) -> u64 {
        self.maximum_depth
    }

    pub const fn scratch_peak_bytes(self) -> u64 {
        self.scratch_peak_bytes
    }
}

/// Adapter boundary between owner-neutral core validation and an operation ledger.
///
/// `Reservation` is a linear RAII token or token bundle. Core holds it for the
/// complete storage lifetime and returns the canonical token to the caller on
/// success. Dropping a failed validation or edit must therefore release every
/// live reservation without a core-to-ledger dependency.
pub trait SceneValidationControl {
    type Error: Error + Send + Sync + 'static;
    type Reservation;

    fn poll(&mut self) -> Result<(), Self::Error>;

    fn charge_work(&mut self, work: SceneValidationWork, amount: u64) -> Result<(), Self::Error>;

    fn reserve(
        &mut self,
        kind: SceneReservationKind,
        bytes: u64,
    ) -> Result<Self::Reservation, Self::Error>;

    /// Resizes one live token without consuming it on failure.
    ///
    /// Core calls this only after a capacity plan has bounded allocator entry.
    /// Implementations must leave `reservation` covering its previous byte
    /// count when returning an error, so the associated allocation remains
    /// accounted while stack unwinding drops it.
    fn resize_reservation(
        &mut self,
        reservation: &mut Self::Reservation,
        kind: SceneReservationKind,
        required_bytes: u64,
    ) -> Result<(), Self::Error>;

    /// Consumes an existing token and returns the replacement classification.
    ///
    /// The returned token must cover at least `required_bytes`. An adapter may
    /// reserve a delta and aggregate multiple underlying tokens, but it must not
    /// count the old and new classifications as simultaneous storage. `from`
    /// and `to` represent distinct lifetime classes; same-class byte changes
    /// must use [`Self::resize_reservation`].
    fn transition_reservation(
        &mut self,
        reservation: Self::Reservation,
        from: SceneReservationKind,
        to: SceneReservationKind,
        required_bytes: u64,
    ) -> Result<Self::Reservation, Self::Error>;
}

/// A central validation failure that preserves adapter-owned causes as sources.
#[derive(Debug, Error)]
pub enum SceneValidationFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Scene(#[from] SceneValidationError),

    #[error("scene validation control failed")]
    Control(#[source] E),
}

/// A validated scene awaiting owner-controlled publication of its canonical reservation.
///
/// The owned scene remains inaccessible until [`Self::try_publish`] consumes the
/// reservation successfully. This prevents callers from cloning or separating a
/// publishable scene while its live storage is still only provisionally reserved.
///
/// ```compile_fail
/// fn extract_early<R>(success: baozi_core::SceneValidationSuccess<R>) {
///     let _ = success.scene();
/// }
/// ```
///
/// ```compile_fail
/// fn split_early<R>(success: baozi_core::SceneValidationSuccess<R>) {
///     let _ = success.into_parts();
/// }
/// ```
#[must_use = "dropping validation success releases its canonical reservation"]
pub struct SceneValidationSuccess<R> {
    scene: ValidatedScene,
    canonical_output: R,
}

impl<R> SceneValidationSuccess<R> {
    pub(crate) fn new(scene: ValidatedScene, canonical_output: R) -> Self {
        Self {
            scene,
            canonical_output,
        }
    }

    pub const fn stats(&self) -> SceneValidationStats {
        self.scene.validation_stats()
    }

    /// Publishes the reservation before releasing the owned scene to the caller.
    ///
    /// If publication fails, both the unpublished scene and the consumed
    /// reservation are discarded before the error is returned.
    pub fn try_publish<E>(
        self,
        publish_reservation: impl FnOnce(R) -> Result<(), E>,
    ) -> Result<ValidatedScene, E> {
        let Self {
            scene,
            canonical_output,
        } = self;
        publish_reservation(canonical_output)?;
        Ok(scene)
    }
}

impl SceneValidationSuccess<()> {
    pub(crate) fn into_unreserved_scene(self) -> ValidatedScene {
        self.scene
    }
}

impl<R> fmt::Debug for SceneValidationSuccess<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SceneValidationSuccess")
            .field("scene", &"<unpublished>")
            .field("stats", &self.scene.validation_stats())
            .field("canonical_output", &"<reservation>")
            .finish()
    }
}

pub(crate) struct NoopSceneValidationControl;

impl SceneValidationControl for NoopSceneValidationControl {
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

    fn transition_reservation(
        &mut self,
        _reservation: Self::Reservation,
        _from: SceneReservationKind,
        _to: SceneReservationKind,
        _required_bytes: u64,
    ) -> Result<Self::Reservation, Self::Error> {
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
}
