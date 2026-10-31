//! Private qualification for retained canonical provenance references.
//!
//! Each edge identifies a local node in one process-unique immutable segment.
//! Global topology indices are derived from the validated segment prefix at lookup.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
thread_local! {
    static ISSUED_PROFILE_SEGMENT_NONCES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static FAIL_NEXT_PROFILE_SEGMENT_NONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static NEXT_PROFILE_REFERENCE_CORRUPTION: std::cell::Cell<Option<ProfileReferenceCorruption>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct ProfileReferenceCorruption {
    references_to_skip: usize,
    component: usize,
    replacement: u64,
}

struct ProfileSegmentNonceSource {
    next: AtomicU64,
}

impl ProfileSegmentNonceSource {
    const fn new(next: u64) -> Self {
        Self {
            next: AtomicU64::new(next),
        }
    }

    fn issue(&self) -> Result<ProfileSegmentNonce, ProfileSegmentNonceExhausted> {
        let candidate = self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |candidate| {
                NonZeroU64::new(candidate).and_then(|_| candidate.checked_add(1))
            })
            .map_err(|_| ProfileSegmentNonceExhausted)?;
        NonZeroU64::new(candidate)
            .map(ProfileSegmentNonce)
            .ok_or(ProfileSegmentNonceExhausted)
    }
}

static PROFILE_SEGMENT_NONCES: ProfileSegmentNonceSource = ProfileSegmentNonceSource::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) struct ProfileSegmentNonce(NonZeroU64);

impl ProfileSegmentNonce {
    pub(in crate::canonical_scene::store) const fn from_stored(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub(in crate::canonical_scene::store) const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct ProfileSegmentNonceExhausted;

pub(in crate::canonical_scene::store) fn issue_profile_segment_nonce()
-> Result<ProfileSegmentNonce, ProfileSegmentNonceExhausted> {
    #[cfg(test)]
    if FAIL_NEXT_PROFILE_SEGMENT_NONCE.with(|flag| flag.replace(false)) {
        return Err(ProfileSegmentNonceExhausted);
    }

    let nonce = PROFILE_SEGMENT_NONCES.issue()?;
    #[cfg(test)]
    ISSUED_PROFILE_SEGMENT_NONCES.with(|count| count.set(count.get().saturating_add(1)));
    Ok(nonce)
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn issued_profile_segment_nonce_count_for_test() -> u64 {
    ISSUED_PROFILE_SEGMENT_NONCES.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn fail_next_profile_segment_nonce_for_test() {
    FAIL_NEXT_PROFILE_SEGMENT_NONCE.with(|flag| flag.set(true));
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn corrupt_profile_reference_for_test(
    references_to_skip: usize,
    component: usize,
    replacement: u64,
) {
    assert!(component < 3);
    assert!(
        component != 1 || replacement != 0,
        "nonce zero is unrepresentable"
    );
    NEXT_PROFILE_REFERENCE_CORRUPTION.with(|slot| {
        assert!(
            slot.replace(Some(ProfileReferenceCorruption {
                references_to_skip,
                component,
                replacement,
            }))
            .is_none()
        );
    });
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn apply_profile_reference_corruption_for_test(
    reference: &mut ProfileProvenanceRef,
) {
    NEXT_PROFILE_REFERENCE_CORRUPTION.with(|slot| {
        let Some(mut corruption) = slot.take() else {
            return;
        };
        if corruption.references_to_skip != 0 {
            corruption.references_to_skip -= 1;
            slot.set(Some(corruption));
            return;
        }
        match corruption.component {
            0 => reference.segment = u32::try_from(corruption.replacement).unwrap_or(u32::MAX),
            1 => {
                reference.nonce = ProfileSegmentNonce::from_stored(corruption.replacement)
                    .expect("test corruption uses a nonzero nonce")
            }
            2 => reference.local = u32::try_from(corruption.replacement).unwrap_or(u32::MAX),
            _ => unreachable!("test corruption selects one of three stored components"),
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct ProfileSegmentAddressDomain {
    ordinal: u32,
    nonce: ProfileSegmentNonce,
    provenance_base: u32,
    provenance_count: u32,
}

/// A logical node in one exact immutable segment. The global index is derived at lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct ProfileProvenanceRef {
    segment: u32,
    nonce: ProfileSegmentNonce,
    local: u32,
}

impl ProfileProvenanceRef {
    pub(in crate::canonical_scene::store) const fn segment(self) -> u32 {
        self.segment
    }

    pub(in crate::canonical_scene::store) const fn local(self) -> u32 {
        self.local
    }

    pub(in crate::canonical_scene::store) fn resolve(
        self,
        domain: ProfileSegmentAddressDomain,
    ) -> Option<u32> {
        (self.segment == domain.ordinal
            && self.nonce == domain.nonce
            && self.local < domain.provenance_count)
            .then(|| domain.provenance_base.checked_add(self.local))
            .flatten()
    }
}

impl ProfileSegmentAddressDomain {
    pub(in crate::canonical_scene::store) const fn new(
        ordinal: u32,
        nonce: ProfileSegmentNonce,
        provenance_base: u32,
        provenance_count: u32,
    ) -> Self {
        Self {
            ordinal,
            nonce,
            provenance_base,
            provenance_count,
        }
    }

    pub(in crate::canonical_scene::store) const fn nonce(self) -> ProfileSegmentNonce {
        self.nonce
    }

    pub(in crate::canonical_scene::store) fn reference(
        self,
        local: u32,
    ) -> Option<ProfileProvenanceRef> {
        if local >= self.provenance_count || self.provenance_base.checked_add(local).is_none() {
            return None;
        }
        #[allow(unused_mut)]
        let mut reference = ProfileProvenanceRef {
            segment: self.ordinal,
            nonce: self.nonce,
            local,
        };
        #[cfg(test)]
        apply_profile_reference_corruption_for_test(&mut reference);
        Some(reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nonce(value: u64) -> ProfileSegmentNonce {
        ProfileSegmentNonce::from_stored(value).expect("test nonce is nonzero")
    }

    #[test]
    fn references_reject_foreign_segments_nonces_and_local_indices() {
        let domain = ProfileSegmentAddressDomain::new(3, nonce(41), 12, 5);
        let valid = domain.reference(2).expect("valid segment-local reference");
        assert_eq!(valid.resolve(domain), Some(14));
        assert_eq!(ProfileSegmentNonce::from_stored(0), None);
        for corrupted in [
            ProfileProvenanceRef {
                segment: 4,
                ..valid
            },
            ProfileProvenanceRef {
                nonce: nonce(42),
                ..valid
            },
            ProfileProvenanceRef { local: 5, ..valid },
        ] {
            assert_eq!(corrupted.resolve(domain), None);
        }
        // Prefixes belong to the validated segment directory, not to each edge.
        assert_eq!(
            valid.resolve(ProfileSegmentAddressDomain::new(3, nonce(41), 100, 5)),
            Some(102)
        );
    }

    #[test]
    fn segment_references_derive_global_indices_and_check_the_local_extent() {
        let domain = ProfileSegmentAddressDomain::new(3, nonce(41), 12, 5);
        for local in 0..5 {
            let reference = domain
                .reference(local)
                .expect("local node belongs to segment");
            assert_eq!(reference.resolve(domain), Some(12 + local));
        }
        assert!(domain.reference(5).is_none());
        assert!(domain.reference(u32::MAX).is_none());
        let overflow = ProfileSegmentAddressDomain::new(0, nonce(42), u32::MAX, 2);
        assert!(overflow.reference(0).is_some());
        assert!(overflow.reference(1).is_none());
    }

    #[test]
    fn issued_segment_nonces_are_nonzero_and_process_unique() {
        let first = issue_profile_segment_nonce().expect("first test nonce");
        let second = issue_profile_segment_nonce().expect("second test nonce");
        assert_ne!(first, second);
        assert_ne!(first.get(), 0);
        assert_ne!(second.get(), 0);
    }

    #[test]
    fn zero_is_an_exhausted_nonce_source() {
        assert_eq!(
            ProfileSegmentNonceSource::new(0).issue(),
            Err(ProfileSegmentNonceExhausted),
        );
    }

    #[test]
    fn issuance_stops_before_the_counter_can_wrap() {
        let source = ProfileSegmentNonceSource::new(u64::MAX - 1);

        assert_eq!(
            source.issue().map(ProfileSegmentNonce::get),
            Ok(u64::MAX - 1),
        );
        assert_eq!(source.issue(), Err(ProfileSegmentNonceExhausted));
        assert_eq!(source.issue(), Err(ProfileSegmentNonceExhausted));
    }

    #[test]
    fn sibling_nonce_substitution_is_rejected() {
        let left_domain = ProfileSegmentAddressDomain::new(1, nonce(51), 7, 2);
        let right_domain = ProfileSegmentAddressDomain::new(1, nonce(52), 7, 2);
        let left_address = left_domain.reference(1).expect("left sibling address");

        assert_eq!(left_address.resolve(left_domain), Some(8));
        assert_eq!(left_address.resolve(right_domain), None);
    }

    #[test]
    fn segment_nonce_exhaustion_is_reported_without_counting_an_issuance() {
        let issued_before = issued_profile_segment_nonce_count_for_test();
        fail_next_profile_segment_nonce_for_test();

        assert_eq!(
            issue_profile_segment_nonce(),
            Err(ProfileSegmentNonceExhausted),
        );
        assert_eq!(issued_profile_segment_nonce_count_for_test(), issued_before);
    }
}
