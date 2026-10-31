use crate::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenErrorKind, AssetOpenStage, AssetOpenWorkError,
    AssetPath, AuthorityDomain, ImmutableMemoryAssetBuildError, OpenedAsset,
};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::num::NonZeroU64;
use std::sync::Arc;

type ImmutableMemoryAssetEntry = (AssetPath, Arc<[u8]>);

const IMMUTABLE_MEMORY_NAMESPACE_DOMAIN: &[u8] = b"baozi.immutable-memory-namespace.v1";

/// An immutable in-memory asset namespace with a stable complete-snapshot digest.
///
/// Construction consumes the complete namespace. Clones share the same frozen entry table and
/// authority identity; no API can add, replace, or remove an entry afterwards.
#[derive(Debug, Clone)]
pub struct ImmutableMemoryAssetIo {
    domain: AuthorityDomain,
    assets: Arc<Vec<ImmutableMemoryAssetEntry>>,
    namespace_snapshot_digest: [u8; 32],
}

impl ImmutableMemoryAssetIo {
    /// Freezes a complete namespace after sorting it by exact normalized path.
    ///
    /// Duplicate paths are rejected rather than resolved by insertion order. Payload bytes remain
    /// shared through their caller-provided `Arc<[u8]>` values.
    pub fn try_from_entries(
        mut assets: Vec<ImmutableMemoryAssetEntry>,
    ) -> Result<Self, ImmutableMemoryAssetBuildError> {
        assets.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        if assets.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(ImmutableMemoryAssetBuildError::DuplicatePath);
        }
        let namespace_snapshot_digest = namespace_snapshot_digest(&assets);
        Ok(Self {
            domain: AuthorityDomain::new(),
            assets: Arc::new(assets),
            namespace_snapshot_digest,
        })
    }

    /// Returns the SHA-256 digest of the complete canonical namespace snapshot.
    pub const fn namespace_snapshot_digest(&self) -> [u8; 32] {
        self.namespace_snapshot_digest
    }

    pub(crate) fn contains(&self, path: &AssetPath) -> bool {
        self.assets
            .binary_search_by(|(candidate, _)| candidate.cmp(path))
            .is_ok()
    }

    pub(crate) const fn authority(&self) -> &AuthorityDomain {
        &self.domain
    }
}

impl AssetIo for ImmutableMemoryAssetIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.domain.clone()
    }

    fn open_work_bound(&self, path: &AssetPath) -> Result<NonZeroU64, AssetOpenWorkError> {
        // The frozen length bounds the search, including its final equality comparison.
        let comparisons = self
            .assets
            .len()
            .checked_ilog2()
            .map_or(0, |depth| depth + 2);
        crate::memory::memory_open_work_bound(path, comparisons)
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        let retained_path = path.try_clone().map_err(|_| {
            AssetOpenError::new(AssetOpenStage::Open, AssetOpenErrorKind::CapacityExceeded)
        })?;
        let index = self
            .assets
            .binary_search_by(|(candidate, _)| candidate.cmp(path))
            .map_err(|_| AssetOpenError::new(AssetOpenStage::Open, AssetOpenErrorKind::NotFound))?;
        let bytes = Arc::clone(&self.assets[index].1);
        let exact_length = u64::try_from(bytes.len()).ok().map(AssetLength::Exact);
        Ok(OpenedAsset::new(
            self.domain.clone(),
            retained_path,
            Cursor::new(bytes),
            exact_length,
        ))
    }
}

fn namespace_snapshot_digest(assets: &[ImmutableMemoryAssetEntry]) -> [u8; 32] {
    let mut digest = Sha256::new();
    write_frame(&mut digest, IMMUTABLE_MEMORY_NAMESPACE_DOMAIN);
    digest.update(encoded_len(assets.len()).to_be_bytes());
    for (path, bytes) in assets {
        write_frame(&mut digest, path.as_str().as_bytes());
        write_frame(&mut digest, bytes);
    }
    digest.finalize().into()
}

fn write_frame(digest: &mut Sha256, bytes: &[u8]) {
    digest.update(encoded_len(bytes.len()).to_be_bytes());
    digest.update(bytes);
}

fn encoded_len(value: usize) -> u64 {
    u64::try_from(value).expect("usize fits in u64 on supported Rust targets")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(path: &str, bytes: &'static [u8]) -> ImmutableMemoryAssetEntry {
        (
            AssetPath::new(path).expect("valid asset path"),
            Arc::from(bytes),
        )
    }

    #[test]
    fn open_work_bound_covers_empty_and_frozen_namespaces_without_changing_reads() {
        let path = AssetPath::new("shared/prefix/target.bin").unwrap();
        let missing = AssetPath::new("shared/prefix/missing.bin").unwrap();
        let empty = ImmutableMemoryAssetIo::try_from_entries(Vec::new()).unwrap();
        let io = ImmutableMemoryAssetIo::try_from_entries(vec![
            asset("shared/prefix/first.bin", b"first"),
            asset("shared/prefix/target.bin", b"target"),
            asset("shared/prefix/z.bin", b"last"),
        ])
        .unwrap();
        let mutable = crate::MemoryAssetIo::new();

        for requested in [&path, &missing] {
            let empty_weight = empty
                .open_work_bound(requested)
                .expect("empty lookup is bounded");
            let weight = io
                .open_work_bound(requested)
                .expect("frozen lookup is bounded");
            assert!(empty_weight.get() >= requested.as_str().len() as u64);
            assert!(weight > empty_weight);
            assert!(weight < mutable.open_work_bound(requested).unwrap());
            assert_eq!(io.clone().open_work_bound(requested), Ok(weight));
            assert_eq!(
                empty.open(requested).unwrap_err().kind(),
                AssetOpenErrorKind::NotFound
            );
        }

        let mut opened = io.open(&path).expect("query preserves the payload");
        let mut bytes = [0_u8; 8];
        assert_eq!(opened.read_chunk(&mut bytes).unwrap(), 6);
        assert_eq!(&bytes[..6], b"target");
        assert_eq!(opened.read_chunk(&mut bytes).unwrap(), 0);
        assert_eq!(
            io.open(&missing).unwrap_err().kind(),
            AssetOpenErrorKind::NotFound
        );
    }

    #[test]
    fn immutable_namespace_is_order_independent_and_shared_by_clones() {
        let first = ImmutableMemoryAssetIo::try_from_entries(vec![
            asset("z.bin", b"last"),
            asset("a.bin", b"first"),
        ])
        .expect("unique namespace");
        let second = ImmutableMemoryAssetIo::try_from_entries(vec![
            asset("a.bin", b"first"),
            asset("z.bin", b"last"),
        ])
        .expect("unique namespace");
        let cloned = first.clone();

        assert_eq!(
            first.namespace_snapshot_digest(),
            second.namespace_snapshot_digest()
        );
        assert_eq!(first.authority_domain(), cloned.authority_domain());
        assert!(first.contains(&AssetPath::new("a.bin").unwrap()));
        assert_eq!(
            first
                .open(&AssetPath::new("missing.bin").unwrap())
                .expect_err("missing input")
                .kind(),
            AssetOpenErrorKind::NotFound
        );
    }

    #[test]
    fn immutable_namespace_rejects_duplicate_paths() {
        let duplicate = AssetPath::new("duplicate.bin").unwrap();
        let result = ImmutableMemoryAssetIo::try_from_entries(vec![
            (duplicate.clone(), Arc::from(&b"first"[..])),
            (duplicate, Arc::from(&b"second"[..])),
        ]);

        assert_eq!(
            result.expect_err("duplicates are ambiguous"),
            ImmutableMemoryAssetBuildError::DuplicatePath
        );
    }
}
