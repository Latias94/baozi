use crate::identity::SceneGenerationId;
use crate::provenance::{OperationIdentity, ProvenanceFootprintError};
use sha2::{Digest, Sha256};
use std::fmt;
use thiserror::Error;

/// The encoded length of a deterministic derived-input stamp.
pub const INPUT_STAMP_BYTES: usize = 32;

/// An opaque deterministic digest of a derived value's declared semantic inputs.
///
/// The producer of a stamp owns the digest algorithm and must include its
/// dependency-schema version in the bytes it hashes. Baozi compares stamps as
/// opaque values and does not infer validity from scene generation alone.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputStamp([u8; INPUT_STAMP_BYTES]);

impl InputStamp {
    pub(crate) const fn from_bytes(bytes: [u8; INPUT_STAMP_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the opaque digest bytes.
    pub const fn as_bytes(&self) -> &[u8; INPUT_STAMP_BYTES] {
        &self.0
    }
}

impl fmt::Debug for InputStamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InputStamp(")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        formatter.write_str(")")
    }
}

impl AsRef<[u8]> for InputStamp {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

const INPUT_STAMP_DOMAIN: &[u8] = b"baozi-input-stamp\0";

/// Versioned, allocation-free SHA-256 framing for declared derived inputs.
pub(crate) struct InputStampBuilder {
    digest: Sha256,
}

impl InputStampBuilder {
    pub(crate) fn new(domain: &[u8], dependency_schema_version: u32) -> Self {
        let mut builder = Self {
            digest: Sha256::new(),
        };
        builder.digest.update(INPUT_STAMP_DOMAIN);
        builder.write_bytes(domain);
        builder.write_u32(dependency_schema_version);
        builder
    }

    pub(crate) fn write_tag(&mut self, value: u8) {
        self.digest.update([value]);
    }

    pub(crate) fn write_u32(&mut self, value: u32) {
        self.digest.update(value.to_le_bytes());
    }

    pub(crate) fn write_u64(&mut self, value: u64) {
        self.digest.update(value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    pub(crate) fn write_f32(&mut self, value: f32) {
        self.write_u32(value.to_bits());
    }

    pub(crate) fn write_f64(&mut self, value: f64) {
        self.write_u64(value.to_bits());
    }

    pub(crate) fn write_bytes(&mut self, value: &[u8]) {
        self.write_usize(value.len());
        self.digest.update(value);
    }

    pub(crate) fn write_bytes_len(&mut self, len: usize) {
        self.write_usize(len);
    }

    pub(crate) fn write_raw_bytes(&mut self, value: &[u8]) {
        self.digest.update(value);
    }

    pub(crate) fn finish(self) -> InputStamp {
        InputStamp::from_bytes(self.digest.finalize().into())
    }
}

/// Why an optional derived value is not present.
///
/// An absence reason is evidence only. An absent value never retains a hidden
/// stale payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DerivedAbsenceReason {
    /// No source or operation supplied the optional value.
    NotProvided,
    /// A committed edit changed declared semantic inputs without recomputing the value.
    Invalidated,
    /// The input cannot support this derived value.
    Unsupported,
    /// The active policy deliberately omitted computation or preservation.
    OmittedByPolicy,
}

#[derive(PartialEq, Eq, Hash)]
enum DerivedProducerState {
    Source,
    Operation(OperationIdentity),
}

/// The producer identity attached to a valid derived value.
///
/// A source producer preserves a value supplied by the input. An operation
/// producer reuses the same canonical operation identity as provenance.
#[derive(PartialEq, Eq, Hash)]
pub struct DerivedProducer {
    state: DerivedProducerState,
}

impl DerivedProducer {
    /// Identifies a value preserved from the source.
    pub const fn source() -> Self {
        Self {
            state: DerivedProducerState::Source,
        }
    }

    /// Consumes and retains the canonical identity of the producing operation.
    pub const fn operation(operation_identity: OperationIdentity) -> Self {
        Self {
            state: DerivedProducerState::Operation(operation_identity),
        }
    }

    /// Returns whether the value was preserved from the source.
    pub const fn is_source(&self) -> bool {
        matches!(&self.state, DerivedProducerState::Source)
    }

    /// Returns the canonical operation identity, or `None` for a source producer.
    pub const fn operation_identity(&self) -> Option<&OperationIdentity> {
        match &self.state {
            DerivedProducerState::Source => None,
            DerivedProducerState::Operation(operation_identity) => Some(operation_identity),
        }
    }

    pub(crate) fn retained_allocation_bytes(&self) -> Result<usize, ProvenanceFootprintError> {
        match &self.state {
            DerivedProducerState::Source => Ok(0),
            DerivedProducerState::Operation(operation_identity) => {
                operation_identity.checked_dynamic_allocation_bytes()
            }
        }
    }
}

impl fmt::Debug for DerivedProducer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.state {
            DerivedProducerState::Source => formatter.write_str("DerivedProducer::Source"),
            DerivedProducerState::Operation(operation_identity) => formatter
                .debug_struct("DerivedProducer::Operation")
                .field("operation_identity", operation_identity)
                .finish(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum DerivedState<T> {
    Absent {
        reason: DerivedAbsenceReason,
    },
    Valid {
        value: T,
        generation: SceneGenerationId,
        input_stamp: InputStamp,
        producer: DerivedProducer,
    },
}

/// An optional derived value with explicit generation and input validity.
///
/// Published values have exactly two states: absent with a reason, or valid
/// with a value and its complete validity evidence. The private representation
/// prevents callers from publishing a third value-bearing invalidated state.
#[derive(Debug, PartialEq, Eq)]
pub struct Derived<T> {
    state: DerivedState<T>,
}

impl<T> Derived<T> {
    /// Creates an absent derived value with no retained payload.
    pub const fn absent(reason: DerivedAbsenceReason) -> Self {
        Self {
            state: DerivedState::Absent { reason },
        }
    }

    /// Returns whether the derived value is absent.
    pub const fn is_absent(&self) -> bool {
        matches!(&self.state, DerivedState::Absent { .. })
    }

    /// Returns whether the derived value has complete validity evidence.
    pub const fn is_valid(&self) -> bool {
        matches!(&self.state, DerivedState::Valid { .. })
    }

    /// Returns the absence reason, or `None` for a valid value.
    pub const fn reason(&self) -> Option<DerivedAbsenceReason> {
        match &self.state {
            DerivedState::Absent { reason } => Some(*reason),
            DerivedState::Valid { .. } => None,
        }
    }

    /// Returns the derived payload only when it is valid.
    pub const fn value(&self) -> Option<&T> {
        match &self.state {
            DerivedState::Absent { .. } => None,
            DerivedState::Valid { value, .. } => Some(value),
        }
    }

    /// Returns the owning scene generation only when the value is valid.
    pub const fn generation(&self) -> Option<SceneGenerationId> {
        match &self.state {
            DerivedState::Absent { .. } => None,
            DerivedState::Valid { generation, .. } => Some(*generation),
        }
    }

    /// Returns the declared-input stamp only when the value is valid.
    pub const fn input_stamp(&self) -> Option<&InputStamp> {
        match &self.state {
            DerivedState::Absent { .. } => None,
            DerivedState::Valid { input_stamp, .. } => Some(input_stamp),
        }
    }

    /// Returns the producer identity only when the value is valid.
    pub const fn producer(&self) -> Option<&DerivedProducer> {
        match &self.state {
            DerivedState::Absent { .. } => None,
            DerivedState::Valid { producer, .. } => Some(producer),
        }
    }

    pub(crate) fn retained_producer_allocation_bytes(
        &self,
    ) -> Result<usize, ProvenanceFootprintError> {
        self.producer()
            .map_or(Ok(0), DerivedProducer::retained_allocation_bytes)
    }

    pub(crate) fn valid(
        value: T,
        generation: SceneGenerationId,
        input_stamp: InputStamp,
        producer: DerivedProducer,
    ) -> Self {
        Self {
            state: DerivedState::Valid {
                value,
                generation,
                input_stamp,
                producer,
            },
        }
    }

    pub(crate) fn rebind_generation(
        &mut self,
        expected_from: SceneGenerationId,
        to: SceneGenerationId,
        expected_stamp: InputStamp,
    ) -> Result<(), DerivedRebindError> {
        let (generation, input_stamp) = match &mut self.state {
            DerivedState::Absent { reason } => {
                return Err(DerivedRebindError::Absent { reason: *reason });
            }
            DerivedState::Valid {
                generation,
                input_stamp,
                ..
            } => (generation, input_stamp),
        };

        if *generation != expected_from {
            return Err(DerivedRebindError::GenerationMismatch {
                expected: expected_from,
                actual: *generation,
            });
        }
        if *input_stamp != expected_stamp {
            return Err(DerivedRebindError::InputStampMismatch {
                expected: expected_stamp,
                actual: *input_stamp,
            });
        }

        *generation = to;
        Ok(())
    }

    pub(crate) fn invalidate(self, reason: DerivedAbsenceReason) -> Self {
        drop(self);
        Self::absent(reason)
    }
}

impl<T> Default for Derived<T> {
    fn default() -> Self {
        Self::absent(DerivedAbsenceReason::NotProvided)
    }
}

/// A failed attempt to preserve valid derived data across scene generations.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DerivedRebindError {
    /// The field had no valid payload to preserve.
    #[error("cannot rebind an absent derived value")]
    Absent { reason: DerivedAbsenceReason },

    /// The field did not belong to the expected input scene generation.
    #[error("derived value belongs to an unexpected scene generation")]
    GenerationMismatch {
        expected: SceneGenerationId,
        actual: SceneGenerationId,
    },

    /// The field's declared semantic inputs did not match the preservation proof.
    #[error("derived value input stamp does not match the expected inputs")]
    InputStampMismatch {
        expected: InputStamp,
        actual: InputStamp,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::{OperationIdentity, OperationKind, OptionsDigest};
    use std::cell::Cell;
    use std::rc::Rc;

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn marker() {}
                }

                impl<T: ?Sized> AmbiguousIfClone<()> for T {}

                struct ImplementsClone;
                impl<T: ?Sized + Clone> AmbiguousIfClone<ImplementsClone> for T {}

                let _ = <$type as AmbiguousIfClone<_>>::marker;
            };
        };
    }

    assert_not_clone!(DerivedProducerState);
    assert_not_clone!(DerivedProducer);
    assert_not_clone!(DerivedState<Vec<u8>>);
    assert_not_clone!(Derived<Vec<u8>>);

    fn generation() -> SceneGenerationId {
        SceneGenerationId::allocate().expect("generation space should remain available")
    }

    fn stamp(byte: u8) -> InputStamp {
        InputStamp::from_bytes([byte; INPUT_STAMP_BYTES])
    }

    #[test]
    fn input_stamp_builder_is_domain_version_and_frame_sensitive() {
        fn build(domain: &[u8], version: u32, parts: &[&[u8]]) -> InputStamp {
            let mut builder = InputStampBuilder::new(domain, version);
            for part in parts {
                builder.write_bytes(part);
            }
            builder.finish()
        }

        let baseline = build(b"primitive-bounds", 1, &[b"ab", b"c"]);
        assert_eq!(baseline, build(b"primitive-bounds", 1, &[b"ab", b"c"]));
        assert_ne!(baseline, build(b"logical-bounds", 1, &[b"ab", b"c"]));
        assert_ne!(baseline, build(b"primitive-bounds", 2, &[b"ab", b"c"]));
        assert_ne!(baseline, build(b"primitive-bounds", 1, &[b"a", b"bc"]));
        assert_ne!(baseline, build(b"primitive-bounds", 1, &[b"abc"]));
    }

    #[test]
    fn source_value_is_valid_only_with_complete_evidence() {
        let generation = generation();
        let input_stamp = stamp(7);
        let derived = Derived::valid(42_u32, generation, input_stamp, DerivedProducer::source());

        assert!(derived.is_valid());
        assert!(!derived.is_absent());
        assert_eq!(derived.value(), Some(&42));
        assert_eq!(derived.generation(), Some(generation));
        assert_eq!(derived.input_stamp(), Some(&input_stamp));
        assert!(derived.producer().is_some_and(DerivedProducer::is_source));
        assert_eq!(derived.reason(), None);
    }

    #[test]
    fn wrong_generation_precedes_input_stamp_mismatch() {
        let actual_generation = generation();
        let wrong_generation = generation();
        let output_generation = generation();
        let original_stamp = stamp(1);
        let mut derived = Derived::valid(
            "bounds",
            actual_generation,
            original_stamp,
            DerivedProducer::source(),
        );

        assert_eq!(
            derived.rebind_generation(wrong_generation, output_generation, stamp(2)),
            Err(DerivedRebindError::GenerationMismatch {
                expected: wrong_generation,
                actual: actual_generation,
            })
        );
        assert_eq!(derived.generation(), Some(actual_generation));
        assert_eq!(derived.input_stamp(), Some(&original_stamp));
    }

    #[test]
    fn input_stamp_mismatch_does_not_rebind() {
        let input_generation = generation();
        let output_generation = generation();
        let original_stamp = stamp(3);
        let expected_stamp = stamp(4);
        let mut derived = Derived::valid(
            "normals",
            input_generation,
            original_stamp,
            DerivedProducer::source(),
        );

        assert_eq!(
            derived.rebind_generation(input_generation, output_generation, expected_stamp),
            Err(DerivedRebindError::InputStampMismatch {
                expected: expected_stamp,
                actual: original_stamp,
            })
        );
        assert_eq!(derived.generation(), Some(input_generation));
        assert_eq!(derived.value(), Some(&"normals"));
    }

    #[test]
    fn operation_producer_consumes_and_retains_identity() {
        let input_generation = generation();
        let output_generation = generation();
        let input_stamp = stamp(5);
        let options_digest = OptionsDigest::sha256([9; 32]);
        let operation_identity =
            OperationIdentity::without_evidence(OperationKind::Transform, options_digest);
        let producer = DerivedProducer::operation(operation_identity);
        let mut derived = Derived::valid(17_u64, input_generation, input_stamp, producer);

        derived
            .rebind_generation(input_generation, output_generation, input_stamp)
            .expect("matching preservation evidence should rebind");

        assert_eq!(derived.value(), Some(&17));
        assert_eq!(derived.generation(), Some(output_generation));
        assert_eq!(derived.input_stamp(), Some(&input_stamp));
        let producer = derived.producer().expect("valid value has a producer");
        let operation_identity = producer
            .operation_identity()
            .expect("operation producer retains its identity");
        assert_eq!(operation_identity.kind(), &OperationKind::Transform);
        assert_eq!(operation_identity.options_digest(), &options_digest);
    }

    #[test]
    fn invalidation_drops_the_payload_and_publishes_absence() {
        #[derive(Debug)]
        struct DropProbe(Rc<Cell<bool>>);

        impl Drop for DropProbe {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let dropped = Rc::new(Cell::new(false));
        let derived = Derived::valid(
            DropProbe(Rc::clone(&dropped)),
            generation(),
            stamp(6),
            DerivedProducer::source(),
        );

        let absent = derived.invalidate(DerivedAbsenceReason::Invalidated);

        assert!(dropped.get(), "invalidation must release the stale payload");
        assert!(absent.is_absent());
        assert_eq!(absent.reason(), Some(DerivedAbsenceReason::Invalidated));
        assert!(absent.value().is_none());
        assert!(absent.generation().is_none());
        assert!(absent.input_stamp().is_none());
        assert!(absent.producer().is_none());
    }
}
