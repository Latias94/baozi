use crate::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenErrorKind, AssetOpenStage, AssetPath,
    AuthorityDomain, MemoryAssetInsertError, OpenedAsset,
};
use std::io::Cursor;
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
