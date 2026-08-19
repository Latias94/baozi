use crate::{
    AssetIo, AssetLength, AssetOpenError, AssetOpenErrorKind, AssetOpenStage, AssetPath,
    AuthorityDomain, ImmutableMemoryAssetBuildError, OpenedAsset,
};
use sha2::{Digest, Sha256};
use std::io::Cursor;
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
