use crate::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenErrorKind, AssetOpenStage, AssetOpenWorkError,
    AssetPath, AuthorityDomain, MemoryAssetInsertError, OpenedAsset,
};
use std::io::Cursor;
use std::num::NonZeroU64;
use std::sync::{Arc, RwLock};

type MemoryAssetEntry = (AssetPath, Arc<[u8]>);

#[derive(Debug, Clone)]
pub struct MemoryAssetIo {
    domain: AuthorityDomain,
    assets: Arc<RwLock<Vec<MemoryAssetEntry>>>,
}

impl Default for MemoryAssetIo {
    fn default() -> Self {
        Self {
            domain: AuthorityDomain::new(),
            assets: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

impl MemoryAssetIo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces one immutable payload in this shared authority namespace.
    ///
    /// New index capacity is allocated before the namespace is changed. Callers
    /// must construct the `Arc<[u8]>` explicitly so a potentially large payload
    /// copy is never hidden inside this operation.
    pub fn try_insert(
        &self,
        path: AssetPath,
        bytes: Arc<[u8]>,
    ) -> Result<Option<Arc<[u8]>>, MemoryAssetInsertError> {
        let mut assets = self
            .assets
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match assets.binary_search_by(|(candidate, _)| candidate.cmp(&path)) {
            Ok(index) => Ok(Some(std::mem::replace(&mut assets[index].1, bytes))),
            Err(index) => {
                try_reserve_entries(&mut assets, 1)?;
                assets.insert(index, (path, bytes));
                Ok(None)
            }
        }
    }
}

fn try_reserve_entries(
    entries: &mut Vec<MemoryAssetEntry>,
    additional: usize,
) -> Result<(), MemoryAssetInsertError> {
    entries
        .try_reserve_exact(additional)
        .map_err(|_| MemoryAssetInsertError::AllocationFailed)
}

impl AssetIo for MemoryAssetIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        // Cover every possible future table size without locking or observing the namespace.
        memory_open_work_bound(path, usize::BITS + 1)
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        let retained_path = path.try_clone().map_err(|_| {
            AssetOpenError::new(AssetOpenStage::Open, AssetOpenErrorKind::CapacityExceeded)
        })?;
        let bytes = {
            let assets = self
                .assets
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let index = assets
                .binary_search_by(|(candidate, _)| candidate.cmp(path))
                .map_err(|_| {
                    AssetOpenError::new(AssetOpenStage::Open, AssetOpenErrorKind::NotFound)
                })?;
            Arc::clone(&assets[index].1)
        };
        let exact_length = u64::try_from(bytes.len()).ok().map(AssetLength::Exact);
        Ok(OpenedAsset::new(
            self.domain.clone(),
            retained_path,
            Cursor::new(bytes),
            exact_length,
        ))
    }
}

pub(crate) fn memory_open_work_bound(
    path: &AssetPath,
    comparisons: u32,
) -> Result<NonZeroU64, AssetOpenWorkError> {
    // One path clone and at most `comparisons` lexicographic comparisons. Each comparison
    // bills at most path.len() byte pairs plus one length/control action, regardless of
    // the candidate's length. Eight fixed units cover lock/Arc/reader setup actions.
    let path_actions = u64::try_from(path.as_str().len())
        .ok()
        .and_then(|bytes| bytes.checked_add(1))
        .ok_or(AssetOpenWorkError::Overflow)?;
    path_actions
        .checked_mul(u64::from(comparisons) + 1)
        .and_then(|weight| weight.checked_add(8))
        .and_then(NonZeroU64::new)
        .ok_or(AssetOpenWorkError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(mut asset: OpenedAsset) -> Vec<u8> {
        let mut output = Vec::new();
        let mut chunk = [0_u8; 16];
        loop {
            let read = asset.read_chunk(&mut chunk).expect("memory read");
            if read == 0 {
                return output;
            }
            output.extend_from_slice(&chunk[..read]);
        }
    }

    #[test]
    fn open_work_bound_is_positive_for_present_missing_and_maximum_length_paths() {
        let io = MemoryAssetIo::new();
        let present = AssetPath::new("present.bin").expect("valid path");
        let missing = AssetPath::new("missing.bin").expect("valid path");
        let longest = AssetPath::new("x".repeat(AssetPath::MAX_BYTES)).expect("valid long path");
        io.try_insert(present.clone(), Arc::from(&b"payload"[..]))
            .expect("asset inserted");

        for path in [&present, &missing, &longest] {
            let weight = io.open_work_bound(path).expect("memory open is bounded");
            assert!(weight.get() >= path.as_str().len() as u64);
            assert_eq!(io.clone().open_work_bound(path), Ok(weight));
        }
        assert!(io.open_work_bound(&longest).unwrap() > io.open_work_bound(&present).unwrap());
        assert_eq!(
            read_all(io.open(&present).expect("present asset")),
            b"payload"
        );
        assert_eq!(
            io.open(&missing)
                .expect_err("query does not add assets")
                .kind(),
            AssetOpenErrorKind::NotFound
        );
    }

    #[test]
    fn open_work_bound_survives_insertion_from_another_shared_handle() {
        let io = MemoryAssetIo::new();
        let path = AssetPath::new("shared/prefix/target.bin").expect("valid path");
        let missing = AssetPath::new("shared/prefix/missing.bin").expect("valid path");
        let weight = io
            .open_work_bound(&path)
            .expect("empty namespace is bounded");
        let missing_weight = io
            .open_work_bound(&missing)
            .expect("missing path is bounded");
        assert_eq!(
            io.open(&path).expect_err("not inserted yet").kind(),
            AssetOpenErrorKind::NotFound
        );

        let writer = io.clone();
        let inserted_path = path.clone();
        std::thread::spawn(move || {
            for index in 0..256 {
                writer
                    .try_insert(
                        AssetPath::new(format!("shared/prefix/entry-{index:03}.bin")).unwrap(),
                        Arc::from(&b"other"[..]),
                    )
                    .expect("concurrent namespace growth");
            }
            writer
                .try_insert(inserted_path, Arc::from(&b"inserted later"[..]))
                .expect("target inserted");
        })
        .join()
        .expect("insertion thread completes");

        assert_eq!(io.open_work_bound(&path), Ok(weight));
        assert_eq!(io.open_work_bound(&missing), Ok(missing_weight));
        let opened = io.open(&path).expect("previously quoted open succeeds");
        io.try_insert(path.clone(), Arc::from(&b"replacement"[..]))
            .expect("target replaced");
        assert_eq!(io.open_work_bound(&path), Ok(weight));
        assert_eq!(read_all(opened), b"inserted later");
        assert_eq!(read_all(io.open(&path).unwrap()), b"replacement");
        assert_eq!(
            io.open(&missing).unwrap_err().kind(),
            AssetOpenErrorKind::NotFound
        );
    }

    #[test]
    fn opens_inserted_memory_asset_with_stable_authority() {
        let path = AssetPath::new("models/triangle.stl").expect("valid path");
        let io = MemoryAssetIo::new();
        io.try_insert(path.clone(), Arc::from(&b"solid test"[..]))
            .expect("asset inserted");
        let cloned = io.clone();

        let opened = cloned.open(&path).expect("asset opens");

        assert_eq!(opened.length(), Some(AssetLength::Exact(10)));
        assert_eq!(opened.authority_domain(), &io.authority_domain());
        assert_eq!(read_all(opened), b"solid test");
    }

    #[test]
    fn clones_share_namespace_mutations_but_independent_namespaces_do_not_alias() {
        let path = AssetPath::new("mutable.bin").expect("valid path");
        let first = MemoryAssetIo::new();
        let observer = first.clone();
        first
            .try_insert(path.clone(), Arc::from(&b"first"[..]))
            .expect("asset inserted");

        let opened_before_update = observer.open(&path).expect("shared asset");

        first
            .try_insert(path.clone(), Arc::from(&b"second"[..]))
            .expect("asset replaced");
        assert_eq!(read_all(opened_before_update), b"first");
        assert_eq!(
            read_all(observer.open(&path).expect("shared update")),
            b"second"
        );

        let independent = MemoryAssetIo::new();
        assert_ne!(first.authority_domain(), independent.authority_domain());
        assert_eq!(
            independent.open(&path).expect_err("missing asset").kind(),
            AssetOpenErrorKind::NotFound
        );
    }

    #[test]
    fn insertion_is_sorted_and_replacement_does_not_grow_the_index() {
        let io = MemoryAssetIo::new();
        let first = Arc::<[u8]>::from(&b"first"[..]);

        for name in ["z.bin", "a.bin", "m.bin"] {
            io.try_insert(
                AssetPath::new(name).expect("valid path"),
                Arc::clone(&first),
            )
            .expect("asset inserted");
        }

        let capacity_before_replacement = io
            .assets
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .capacity();
        let previous = io
            .try_insert(
                AssetPath::new("m.bin").expect("valid path"),
                Arc::from(&b"replacement"[..]),
            )
            .expect("asset replaced")
            .expect("previous payload");
        let assets = io
            .assets
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        assert_eq!(previous.as_ref(), b"first");
        assert_eq!(assets.capacity(), capacity_before_replacement);
        assert_eq!(
            assets
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            ["a.bin", "m.bin", "z.bin"]
        );
    }

    #[test]
    fn allocation_failure_is_typed_and_leaves_entries_unchanged() {
        let mut entries = Vec::new();

        let error = try_reserve_entries(&mut entries, usize::MAX)
            .expect_err("impossible capacity must fail");

        assert_eq!(error, MemoryAssetInsertError::AllocationFailed);
        assert!(entries.is_empty());
    }
}
