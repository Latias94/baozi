//! Private qualification for retained canonical provenance references.
//!
//! A global topology index alone cannot distinguish two sibling segments that
//! happen to append the same local node. Every stored edge therefore carries
//! the segment ordinal, its process-unique nonce, the segment-local index, and
//! the generation-wide topology index. This module is the only place that
//! constructs or validates that four-part address.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

pub(in crate::canonical_scene::store) const PROFILE_PROVENANCE_ADDRESS_SCALARS: usize = 4;
pub(in crate::canonical_scene::store) const PROFILE_PROVENANCE_ADDRESS_LOGICAL_BYTES: u64 =
    PROFILE_PROVENANCE_ADDRESS_SCALARS as u64 * 8;

#[cfg(test)]
thread_local! {
    static ISSUED_PROFILE_SEGMENT_NONCES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static FAIL_NEXT_PROFILE_SEGMENT_NONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static NEXT_PROFILE_ADDRESS_CORRUPTION: std::cell::Cell<Option<ProfileAddressCorruption>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct ProfileAddressCorruption {
    addresses_to_skip: usize,
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
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |candidate| {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::canonical_scene::store) struct QualifiedProvenanceAddress {
    segment_ordinal: u64,
    segment_nonce: ProfileSegmentNonce,
    local_index: u64,
    global_index: u64,
}

impl QualifiedProvenanceAddress {
    pub(in crate::canonical_scene::store) fn from_scalars(scalars: &[u64]) -> Option<Self> {
        let scalars: &[u64; PROFILE_PROVENANCE_ADDRESS_SCALARS] = scalars.try_into().ok()?;
        let [segment_ordinal, segment_nonce, local_index, global_index] = *scalars;
        Some(Self {
            segment_ordinal,
            segment_nonce: ProfileSegmentNonce::from_stored(segment_nonce)?,
            local_index,
            global_index,
        })
    }

    pub(in crate::canonical_scene::store) fn into_scalars(
        self,
    ) -> [u64; PROFILE_PROVENANCE_ADDRESS_SCALARS] {
        let scalars = [
            self.segment_ordinal,
            self.segment_nonce.get(),
            self.local_index,
            self.global_index,
        ];
        #[cfg(test)]
        let scalars = {
            let mut scalars = scalars;
            apply_next_profile_address_corruption_for_test(&mut scalars);
            scalars
        };
        scalars
    }

    pub(in crate::canonical_scene::store) fn segment_ordinal(self) -> Option<u32> {
        u32::try_from(self.segment_ordinal).ok()
    }

    pub(in crate::canonical_scene::store) fn global_index(self) -> Option<u32> {
        u32::try_from(self.global_index).ok()
    }

    pub(in crate::canonical_scene::store) fn validate_against(
        self,
        segment: ProfileSegmentAddressDomain,
    ) -> Option<u32> {
        let ordinal = self.segment_ordinal()?;
        let local = u32::try_from(self.local_index).ok()?;
        let global = self.global_index()?;
        (ordinal == segment.ordinal
            && self.segment_nonce == segment.nonce
            && local < segment.provenance_count
            && segment.provenance_base.checked_add(local) == Some(global))
        .then_some(global)
    }
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn corrupt_profile_address_for_test(
    addresses_to_skip: usize,
    component: usize,
    replacement: u64,
) {
    assert!(component < PROFILE_PROVENANCE_ADDRESS_SCALARS);
    NEXT_PROFILE_ADDRESS_CORRUPTION.with(|corruption| {
        assert!(
            corruption
                .replace(Some(ProfileAddressCorruption {
                    addresses_to_skip,
                    component,
                    replacement,
                }))
                .is_none()
        );
    });
}

#[cfg(test)]
fn apply_next_profile_address_corruption_for_test(
    scalars: &mut [u64; PROFILE_PROVENANCE_ADDRESS_SCALARS],
) {
    NEXT_PROFILE_ADDRESS_CORRUPTION.with(|slot| {
        let Some(mut corruption) = slot.take() else {
            return;
        };
        if corruption.addresses_to_skip != 0 {
            corruption.addresses_to_skip -= 1;
            slot.set(Some(corruption));
            return;
        }
        scalars[corruption.component] = corruption.replacement;
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct ProfileSegmentAddressDomain {
    ordinal: u32,
    nonce: ProfileSegmentNonce,
    provenance_base: u32,
    provenance_count: u32,
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

    /// Constructs an address within this exact segment. The caller supplies only a local index;
    /// the domain owns its nonce, extent, and global base, so those cannot drift independently.
    pub(in crate::canonical_scene::store) fn address(
        self,
        local_index: u32,
    ) -> Option<QualifiedProvenanceAddress> {
        if local_index >= self.provenance_count {
            return None;
        }
        let global_index = self.provenance_base.checked_add(local_index)?;
        Some(QualifiedProvenanceAddress {
            segment_ordinal: u64::from(self.ordinal),
            segment_nonce: self.nonce,
            local_index: u64::from(local_index),
            global_index: u64::from(global_index),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct QualifiedProvenanceAddressSlice<'values> {
    scalars: &'values [u64],
}

impl<'values> QualifiedProvenanceAddressSlice<'values> {
    pub(in crate::canonical_scene::store) fn from_scalars(
        scalars: &'values [u64],
        address_count: usize,
    ) -> Option<Self> {
        let expected = address_count.checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)?;
        (scalars.len() == expected).then_some(Self { scalars })
    }

    pub(in crate::canonical_scene::store) const fn len(self) -> usize {
        self.scalars.len() / PROFILE_PROVENANCE_ADDRESS_SCALARS
    }

    pub(in crate::canonical_scene::store) fn get_range(
        self,
        start: usize,
        count: usize,
    ) -> Option<Self> {
        let scalar_start = start.checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)?;
        let scalar_count = count.checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)?;
        let scalar_end = scalar_start.checked_add(scalar_count)?;
        Self::from_scalars(self.scalars.get(scalar_start..scalar_end)?, count)
    }

    pub(in crate::canonical_scene::store) fn raw_prefix(
        self,
        address_count: usize,
    ) -> Option<&'values [u64]> {
        let end = address_count.checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)?;
        self.scalars.get(..end)
    }

    pub(in crate::canonical_scene::store) fn raw_suffix(
        self,
        address_start: usize,
    ) -> Option<&'values [u64]> {
        let start = address_start.checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)?;
        self.scalars.get(start..)
    }

    pub(in crate::canonical_scene::store) fn iter(
        self,
    ) -> impl ExactSizeIterator<Item = Option<QualifiedProvenanceAddress>> + 'values {
        self.scalars
            .chunks_exact(PROFILE_PROVENANCE_ADDRESS_SCALARS)
            .map(QualifiedProvenanceAddress::from_scalars)
    }

    pub(in crate::canonical_scene::store) fn global_indices(
        self,
    ) -> impl ExactSizeIterator<Item = Option<u32>> + 'values {
        self.iter()
            .map(|address| address.and_then(QualifiedProvenanceAddress::global_index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nonce(value: u64) -> ProfileSegmentNonce {
        ProfileSegmentNonce::from_stored(value).expect("test nonce is nonzero")
    }

    #[test]
    fn qualified_address_rejects_each_independently_corrupted_component() {
        let domain = ProfileSegmentAddressDomain::new(3, nonce(41), 12, 5);
        let valid = domain.address(2).expect("valid qualified address");
        assert_eq!(valid.validate_against(domain), Some(14));
        assert!(QualifiedProvenanceAddress::from_scalars(&[3, 0, 2, 14]).is_none());

        for corrupted in [
            [4, 41, 2, 14],
            [3, 42, 2, 14],
            [3, 41, 5, 14],
            [3, 41, 2, 15],
        ] {
            let corrupted = QualifiedProvenanceAddress::from_scalars(&corrupted)
                .expect("corruption remains structurally decodable");
            assert_eq!(corrupted.validate_against(domain), None);
        }
    }

    #[test]
    fn segment_addresses_derive_global_indices_and_check_the_local_extent() {
        let domain = ProfileSegmentAddressDomain::new(3, nonce(41), 12, 5);
        for local in 0..5 {
            let address = domain
                .address(local)
                .expect("local index belongs to this segment");
            assert_eq!(address.validate_against(domain), Some(12 + local));
            assert_eq!(
                address.into_scalars(),
                [3, 41, u64::from(local), 12 + u64::from(local)]
            );
        }
        assert!(domain.address(5).is_none());
        assert!(domain.address(u32::MAX).is_none());
        let overflow = ProfileSegmentAddressDomain::new(0, nonce(42), u32::MAX, 2);
        assert!(overflow.address(0).is_some());
        assert!(overflow.address(1).is_none());
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
        let left_address = left_domain.address(1).expect("left sibling address");

        assert_eq!(left_address.validate_against(left_domain), Some(8));
        assert_eq!(left_address.validate_against(right_domain), None);
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
