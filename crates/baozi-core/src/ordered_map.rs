use baozi_operation::{CapacityPlan, CapacityPlanError};
use std::borrow::Borrow;
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;

/// A capacity or allocation failure while growing an ordered canonical map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OrderedMapError {
    /// Entry-count or byte-capacity arithmetic overflowed.
    CapacityOverflow,
    /// The backing allocation could not be reserved.
    AllocationFailed,
    /// A caller attempted an inserting operation without reserving capacity first.
    CapacityNotReserved,
    /// The shared capacity policy rejected planning or allocator behavior.
    CapacityPlan(CapacityPlanError),
}

impl fmt::Display for OrderedMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CapacityOverflow => "ordered map capacity accounting overflowed",
            Self::AllocationFailed => "ordered map allocation failed",
            Self::CapacityNotReserved => "ordered map insertion requires reserved capacity",
            Self::CapacityPlan(cause) => return cause.fmt(formatter),
        })
    }
}

impl std::error::Error for OrderedMapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CapacityPlan(cause) => Some(cause),
            Self::CapacityOverflow | Self::AllocationFailed | Self::CapacityNotReserved => None,
        }
    }
}

/// A capacity plan bound to one concrete ordered-map entry type.
///
/// The type parameter prevents a plan admitted for one `(K, V)` layout from
/// being applied to another map with coincidentally equal length and capacity.
pub struct OrderedMapCapacityPlan<K, V> {
    inner: CapacityPlan,
    marker: PhantomData<fn() -> (K, V)>,
}

/// A full-allocation replacement plan that leaves the live map untouched until commit.
pub(crate) struct OrderedMapReplacementPlan<K, V> {
    original_len: usize,
    original_capacity: usize,
    additional: usize,
    replacement: Option<CapacityPlan>,
    marker: PhantomData<fn() -> (K, V)>,
}

/// A fully allocated map insertion candidate whose commit cannot enter the allocator.
pub(crate) struct PreparedOrderedMapInsert<'owner, K, V> {
    owner: &'owner mut OrderedMap<K, V>,
    replacement: Option<Vec<(K, V)>>,
    entry: Option<(K, V)>,
    location: Result<usize, usize>,
    replacement_bytes: u64,
}

impl<K, V> Copy for OrderedMapCapacityPlan<K, V> {}

impl<K, V> Clone for OrderedMapCapacityPlan<K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K, V> fmt::Debug for OrderedMapCapacityPlan<K, V> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OrderedMapCapacityPlan")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<K, V> PartialEq for OrderedMapCapacityPlan<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<K, V> Eq for OrderedMapCapacityPlan<K, V> {}

impl<K, V> OrderedMapCapacityPlan<K, V> {
    /// Returns the byte increase that must be admitted before allocator entry.
    pub fn envelope_delta_bytes(&self) -> Result<u64, OrderedMapError> {
        self.inner
            .envelope_delta_bytes()
            .map_err(OrderedMapError::CapacityPlan)
    }
}

impl<K, V> OrderedMapReplacementPlan<K, V> {
    pub(crate) fn envelope_bytes(&self) -> Result<u64, OrderedMapError> {
        self.replacement.map_or(Ok(0), |plan| {
            plan.envelope_delta_bytes()
                .map_err(OrderedMapError::CapacityPlan)
        })
    }

    pub(crate) const fn replaces_allocation(&self) -> bool {
        self.replacement.is_some()
    }
}

/// A deterministic map backed by one explicitly reservable contiguous allocation.
///
/// Keys stay sorted, so iteration and equality are deterministic. Growth is split
/// into prediction, fallible reservation, and allocation-free insertion for
/// callers that must admit live bytes before entering the allocator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedMap<K, V> {
    entries: Vec<(K, V)>,
}

impl<K, V> OrderedMap<K, V> {
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub const fn capacity(&self) -> usize {
        self.entries.capacity()
    }

    pub fn capacity_bytes(&self) -> Result<u64, OrderedMapError> {
        capacity_bytes::<K, V>(self.capacity())
    }

    /// Plans a deterministic capacity envelope for `additional` new entries.
    pub fn capacity_plan(
        &self,
        additional: usize,
    ) -> Result<OrderedMapCapacityPlan<K, V>, OrderedMapError> {
        let desired_capacity = self
            .len()
            .checked_add(additional)
            .ok_or(OrderedMapError::CapacityOverflow)?;
        let inner = CapacityPlan::for_type::<(K, V)>(self.len(), self.capacity(), desired_capacity)
            .map_err(OrderedMapError::CapacityPlan)?;
        Ok(OrderedMapCapacityPlan {
            inner,
            marker: PhantomData,
        })
    }

    /// Plans a complete replacement allocation when the current map has no free insertion slot.
    pub(crate) fn replacement_plan(
        &self,
        additional: usize,
    ) -> Result<OrderedMapReplacementPlan<K, V>, OrderedMapError> {
        let required_capacity = self
            .len()
            .checked_add(additional)
            .ok_or(OrderedMapError::CapacityOverflow)?;
        let replacement = if required_capacity <= self.capacity() {
            None
        } else {
            Some(
                CapacityPlan::for_type::<(K, V)>(0, 0, required_capacity)
                    .map_err(OrderedMapError::CapacityPlan)?,
            )
        };
        Ok(OrderedMapReplacementPlan {
            original_len: self.len(),
            original_capacity: self.capacity(),
            additional,
            replacement,
            marker: PhantomData,
        })
    }

    /// Allocates a replacement without changing the live map or moving the entry into it.
    pub(crate) fn prepare_replacement_insert(
        &mut self,
        plan: OrderedMapReplacementPlan<K, V>,
        key: K,
        value: V,
    ) -> Result<PreparedOrderedMapInsert<'_, K, V>, OrderedMapError>
    where
        K: Ord,
    {
        if self.len() != plan.original_len || self.capacity() != plan.original_capacity {
            return Err(OrderedMapError::CapacityPlan(
                CapacityPlanError::StalePlan {
                    expected_len: plan.original_len,
                    expected_capacity: plan.original_capacity,
                    actual_len: self.len(),
                    actual_capacity: self.capacity(),
                },
            ));
        }

        let location = self.find(&key);
        let required_additional = usize::from(location.is_err());
        if required_additional > plan.additional {
            return Err(OrderedMapError::CapacityPlan(
                CapacityPlanError::StalePlan {
                    expected_len: plan.original_len.saturating_add(plan.additional),
                    expected_capacity: plan.original_capacity,
                    actual_len: self.len().saturating_add(required_additional),
                    actual_capacity: self.capacity(),
                },
            ));
        }

        let (replacement, replacement_bytes) = match plan.replacement {
            Some(replacement_plan) => {
                replacement_plan
                    .verify_preflight(0, 0)
                    .map_err(OrderedMapError::CapacityPlan)?;
                let mut replacement = Vec::new();
                replacement
                    .try_reserve_exact(replacement_plan.allocator_additional())
                    .map_err(|_| OrderedMapError::AllocationFailed)?;
                let replacement_bytes = replacement_plan
                    .finish(replacement.len(), replacement.capacity())
                    .map_err(OrderedMapError::CapacityPlan)?;
                (Some(replacement), replacement_bytes)
            }
            None => {
                if location.is_err() && self.len() == self.capacity() {
                    return Err(OrderedMapError::CapacityNotReserved);
                }
                (None, 0)
            }
        };

        Ok(PreparedOrderedMapInsert {
            owner: self,
            replacement,
            entry: Some((key, value)),
            location,
            replacement_bytes,
        })
    }

    /// Applies a previously admitted plan and returns actual retained growth.
    pub fn apply_capacity_plan(
        &mut self,
        plan: OrderedMapCapacityPlan<K, V>,
    ) -> Result<u64, OrderedMapError> {
        plan.inner
            .verify_preflight(self.len(), self.capacity())
            .map_err(OrderedMapError::CapacityPlan)?;
        self.entries
            .try_reserve_exact(plan.inner.allocator_additional())
            .map_err(|_| OrderedMapError::AllocationFailed)?;
        plan.inner
            .finish(self.len(), self.capacity())
            .map_err(OrderedMapError::CapacityPlan)
    }

    pub fn iter(&self) -> OrderedMapIter<'_, K, V> {
        OrderedMapIter(self.entries.iter())
    }

    pub fn keys(&self) -> impl ExactSizeIterator<Item = &K> {
        self.entries.iter().map(|(key, _)| key)
    }

    pub fn values(&self) -> impl ExactSizeIterator<Item = &V> {
        self.entries.iter().map(|(_, value)| value)
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
    {
        self.find(key).ok().map(|index| &self.entries[index].1)
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
    {
        let index = self.find(key).ok()?;
        Some(&mut self.entries[index].1)
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
    {
        self.find(key).is_ok()
    }

    /// Fallibly grows as needed and inserts one value.
    ///
    /// Import and post-process code should use the explicit two-phase API so
    /// its ledger reservation precedes [`Self::apply_capacity_plan`].
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, OrderedMapError>
    where
        K: Ord,
    {
        match self.find(&key) {
            Ok(index) => Ok(Some(std::mem::replace(&mut self.entries[index].1, value))),
            Err(index) => {
                let plan = self.capacity_plan(1)?;
                self.apply_capacity_plan(plan)?;
                self.entries.insert(index, (key, value));
                Ok(None)
            }
        }
    }

    /// Inserts without allocation after the caller has admitted and reserved capacity.
    pub fn insert_reserved(&mut self, key: K, value: V) -> Result<Option<V>, OrderedMapError>
    where
        K: Ord,
    {
        match self.find(&key) {
            Ok(index) => Ok(Some(std::mem::replace(&mut self.entries[index].1, value))),
            Err(index) => {
                if self.len() == self.capacity() {
                    return Err(OrderedMapError::CapacityNotReserved);
                }
                self.entries.insert(index, (key, value));
                Ok(None)
            }
        }
    }

    fn find<Q>(&self, key: &Q) -> Result<usize, usize>
    where
        K: Borrow<Q> + Ord,
        Q: Ord + ?Sized,
    {
        self.entries
            .binary_search_by(|(candidate, _)| candidate.borrow().cmp(key))
    }
}

impl<K, V> PreparedOrderedMapInsert<'_, K, V> {
    pub(crate) const fn replacement_bytes(&self) -> u64 {
        self.replacement_bytes
    }

    pub(crate) const fn replaces_allocation(&self) -> bool {
        self.replacement.is_some()
    }

    /// Commits by moving values into already admitted storage; no allocator or fallible hook runs.
    pub(crate) fn commit(mut self) -> Option<(K, V)> {
        let (key, value) = self
            .entry
            .take()
            .expect("a prepared ordered-map entry is consumed exactly once");
        match self.location {
            Ok(index) => Some(std::mem::replace(
                &mut self.owner.entries[index],
                (key, value),
            )),
            Err(index) => {
                if let Some(mut replacement) = self.replacement.take() {
                    assert!(
                        replacement.capacity() > self.owner.len(),
                        "a replacement map allocation must admit the inserted entry"
                    );
                    replacement.append(&mut self.owner.entries);
                    replacement.insert(index, (key, value));
                    std::mem::swap(&mut self.owner.entries, &mut replacement);
                    drop(replacement);
                } else {
                    assert!(
                        self.owner.len() < self.owner.capacity(),
                        "an in-place map insertion requires an admitted slot"
                    );
                    self.owner.entries.insert(index, (key, value));
                }
                None
            }
        }
    }
}

impl<K, V> Default for OrderedMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, K, V> IntoIterator for &'a OrderedMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = OrderedMapIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct OrderedMapIter<'a, K, V>(std::slice::Iter<'a, (K, V)>);

impl<'a, K, V> Iterator for OrderedMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(key, value)| (key, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K, V> ExactSizeIterator for OrderedMapIter<'_, K, V> {}
impl<K, V> std::iter::FusedIterator for OrderedMapIter<'_, K, V> {}

impl<K, V> IntoIterator for OrderedMap<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

fn capacity_bytes<K, V>(capacity: usize) -> Result<u64, OrderedMapError> {
    let bytes = capacity
        .checked_mul(size_of::<(K, V)>())
        .ok_or(OrderedMapError::CapacityOverflow)?;
    u64::try_from(bytes).map_err(|_| OrderedMapError::CapacityOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_insert_is_sorted_and_allocation_free() {
        let mut map = OrderedMap::new();
        let plan = map.capacity_plan(2).unwrap();
        let admitted = plan.envelope_delta_bytes().unwrap();
        let actual = map.apply_capacity_plan(plan).unwrap();
        assert!(
            actual <= admitted,
            "allocator exceeded its admitted envelope"
        );
        let capacity = map.capacity();

        map.insert_reserved("z", 1).unwrap();
        map.insert_reserved("a", 2).unwrap();

        assert_eq!(map.capacity(), capacity);
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&"a", &2), (&"z", &1)]);
        assert_eq!(map.get("a"), Some(&2));
    }

    #[test]
    fn unreserved_insert_is_rejected_before_vec_growth() {
        let mut map = OrderedMap::new();
        assert_eq!(
            map.insert_reserved("a", 1),
            Err(OrderedMapError::CapacityNotReserved)
        );
        assert_eq!(map.capacity(), 0);
        assert!(map.is_empty());
    }

    #[test]
    fn replacing_an_existing_key_needs_no_spare_capacity() {
        let mut map = OrderedMap::new();
        map.try_insert("a", 1).unwrap();
        let capacity = map.capacity();
        assert_eq!(map.insert_reserved("a", 2).unwrap(), Some(1));
        assert_eq!(map.capacity(), capacity);
        assert_eq!(map.get("a"), Some(&2));
    }

    #[test]
    fn stale_capacity_plan_is_rejected_before_allocator_entry() {
        let mut map = OrderedMap::new();
        let stale = map.capacity_plan(1).unwrap();
        map.try_insert("a", 1).unwrap();
        let capacity = map.capacity();

        assert!(matches!(
            map.apply_capacity_plan(stale),
            Err(OrderedMapError::CapacityPlan(
                CapacityPlanError::StalePlan { .. }
            ))
        ));
        assert_eq!(map.capacity(), capacity);
    }

    #[test]
    fn prepared_replacement_drop_leaves_the_live_map_unchanged() {
        let mut map = OrderedMap::new();
        map.try_insert("z".to_owned(), 1).unwrap();
        let original = map.clone();
        let plan = map.replacement_plan(1).unwrap();
        let admitted = plan.envelope_bytes().unwrap();

        let prepared = map
            .prepare_replacement_insert(plan, "a".to_owned(), 2)
            .unwrap();
        assert!(prepared.replaces_allocation());
        assert!(prepared.replacement_bytes() <= admitted);
        drop(prepared);

        assert_eq!(map, original);
    }

    #[test]
    fn prepared_replacement_commits_sorted_storage_and_returns_the_retired_entry() {
        let mut map = OrderedMap::new();
        map.try_insert("z".to_owned(), 1).unwrap();

        let insert_plan = map.replacement_plan(1).unwrap();
        let inserted = map
            .prepare_replacement_insert(insert_plan, "a".to_owned(), 2)
            .unwrap()
            .commit();
        assert_eq!(inserted, None);
        assert_eq!(
            map.iter().collect::<Vec<_>>(),
            vec![(&"a".to_owned(), &2), (&"z".to_owned(), &1)]
        );

        let replace_plan = map.replacement_plan(0).unwrap();
        let retired = map
            .prepare_replacement_insert(replace_plan, "a".to_owned(), 3)
            .unwrap()
            .commit();
        assert_eq!(retired, Some(("a".to_owned(), 2)));
        assert_eq!(map.get("a"), Some(&3));
    }
}
