use crate::{AssetOpenError, AssetPath, AssetReadContractViolation, AssetReadError};
use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::panic::RefUnwindSafe;
use std::sync::Arc;

#[derive(Default)]
struct AuthorityIdentity;

/// Opaque identity for one authority namespace.
///
/// Clones compare equal, while independently constructed domains never do. The
/// identity is intentionally process-local and must not enter durable evidence.
#[derive(Clone, Default)]
pub struct AuthorityDomain(Arc<AuthorityIdentity>);

impl AuthorityDomain {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PartialEq for AuthorityDomain {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for AuthorityDomain {}

impl Hash for AuthorityDomain {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

impl fmt::Debug for AuthorityDomain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorityDomain(..)")
    }
}

/// The strength of a byte-length observation made by an [`AssetIo`] backend.
///
/// Exact lengths are enforced by [`OpenedAsset`]. Hints are useful for bounded
/// capacity planning and progress reporting, but they are never an upper bound
/// and must not replace an import-owned hard limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetLength {
    /// An immutable resource contract: EOF must occur after exactly this many bytes.
    Exact(u64),
    /// A same-object observation that may become stale while the object is read.
    Hint(u64),
}

impl AssetLength {
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Exact(bytes) | Self::Hint(bytes) => bytes,
        }
    }

    pub const fn is_exact(self) -> bool {
        matches!(self, Self::Exact(_))
    }
}

/// A one-shot reader owned by an [`OpenedAsset`].
///
/// Returning zero for a non-empty buffer declares terminal EOF. An
/// implementation must never return a byte count larger than `buffer.len()`;
/// [`OpenedAsset`] converts violations into a typed error before callers can
/// slice with the invalid count.
pub trait AssetReader: Send {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError>;
}

impl<T> AssetReader for T
where
    T: Read + Send,
{
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        self.read(buffer).map_err(AssetReadError::from_io)
    }
}

#[derive(Debug)]
enum AssetReadState {
    Active,
    Eof,
    ContractFailed(AssetReadError),
}

/// An already-open resource whose reader remains bound to the authorized object.
pub struct OpenedAsset {
    domain: AuthorityDomain,
    path: AssetPath,
    reader: Box<dyn AssetReader>,
    length: Option<AssetLength>,
    backend_hard_limit: Option<u64>,
    bytes_read: u64,
    read_state: AssetReadState,
}

impl OpenedAsset {
    /// Binds an authorized identity, reader, and optional length contract.
    ///
    /// Custom backends must pass the same authority domain and normalized path
    /// returned by the `AssetIo` operation that produced this object. Import
    /// sessions validate that binding with [`Self::matches_identity`].
    pub fn new(
        domain: AuthorityDomain,
        path: AssetPath,
        reader: impl AssetReader + 'static,
        length: Option<AssetLength>,
    ) -> Self {
        Self {
            domain,
            path,
            reader: Box::new(reader),
            length,
            backend_hard_limit: None,
            bytes_read: 0,
            read_state: AssetReadState::Active,
        }
    }

    /// Adds a backend-owned upper bound that the import layer must intersect
    /// with its request and aggregate limits.
    pub fn with_backend_hard_limit(mut self, maximum_bytes: u64) -> Self {
        self.backend_hard_limit = Some(maximum_bytes);
        self
    }

    pub fn authority_domain(&self) -> &AuthorityDomain {
        &self.domain
    }

    pub fn path(&self) -> &AssetPath {
        &self.path
    }

    pub fn matches_identity(&self, domain: &AuthorityDomain, path: &AssetPath) -> bool {
        &self.domain == domain && &self.path == path
    }

    pub const fn length(&self) -> Option<AssetLength> {
        self.length
    }

    pub const fn backend_hard_limit(&self) -> Option<u64> {
        self.backend_hard_limit
    }

    /// Returns the number of bytes credibly observed from the backend.
    pub const fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    pub fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
        // Match `Read` semantics without confusing an empty request with EOF or
        // an exact-length mismatch.
        if buffer.is_empty() {
            return Ok(0);
        }
        match &self.read_state {
            AssetReadState::Active => {}
            AssetReadState::Eof => return Ok(0),
            AssetReadState::ContractFailed(error) => return Err(error.clone()),
        }

        let count = self.reader.read_chunk(buffer)?;
        if count > buffer.len() {
            return Err(self.latch_contract_violation(
                AssetReadContractViolation::CountExceedsBuffer {
                    returned: count,
                    capacity: buffer.len(),
                },
            ));
        }

        let count_u64 = match u64::try_from(count) {
            Ok(count) => count,
            Err(_) => {
                return Err(self.latch_contract_violation(
                    AssetReadContractViolation::ByteCountOverflow {
                        consumed: self.bytes_read,
                        returned: count,
                    },
                ));
            }
        };
        let next = match self.bytes_read.checked_add(count_u64) {
            Some(next) => next,
            None => {
                return Err(self.latch_contract_violation(
                    AssetReadContractViolation::ByteCountOverflow {
                        consumed: self.bytes_read,
                        returned: count,
                    },
                ));
            }
        };
        self.bytes_read = next;

        if let Some(AssetLength::Exact(expected)) = self.length {
            if next > expected {
                return Err(self.latch_contract_violation(
                    AssetReadContractViolation::ExactLengthExceeded {
                        expected,
                        observed: next,
                    },
                ));
            }
            if count == 0 && next != expected {
                return Err(self.latch_contract_violation(
                    AssetReadContractViolation::ExactLengthTruncated {
                        expected,
                        observed: next,
                    },
                ));
            }
        }

        if count == 0 {
            self.read_state = AssetReadState::Eof;
        }
        Ok(count)
    }

    fn latch_contract_violation(
        &mut self,
        violation: AssetReadContractViolation,
    ) -> AssetReadError {
        let error = AssetReadError::contract_violation(violation);
        self.read_state = AssetReadState::ContractFailed(error.clone());
        error
    }
}

impl fmt::Debug for OpenedAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenedAsset")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("length", &self.length)
            .field("backend_hard_limit", &self.backend_hard_limit)
            .field("bytes_read", &self.bytes_read)
            .field("read_state", &self.read_state)
            .finish_non_exhaustive()
    }
}

/// Reusable authority for opening logical asset identities.
///
/// `authority_domain` must return the same process-local identity for the
/// entire logical lifetime of one backend namespace, including all of its
/// clones and calls made after a caught unwind. Every successful `open` must
/// return an [`OpenedAsset`] that matches that domain and the requested path.
///
/// The `RefUnwindSafe` bound makes the trusted-extension promise explicit:
/// Baozi may catch an unwind from a shared call and later use the capability in
/// another import session. A hostile implementation still requires process
/// isolation; this trait is not a sandbox boundary.
pub trait AssetIo: Any + Send + Sync + RefUnwindSafe + 'static {
    fn authority_domain(&self) -> AuthorityDomain;

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::panic::RefUnwindSafe;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingReader {
        calls: Arc<AtomicUsize>,
        returned_counts: Vec<usize>,
    }

    impl CountingReader {
        fn new(returned_counts: impl Into<Vec<usize>>) -> (Self, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    calls: Arc::clone(&calls),
                    returned_counts: returned_counts.into(),
                },
                calls,
            )
        }
    }

    impl AssetReader for CountingReader {
        fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed);
            let count = self.returned_counts.get(call).copied().unwrap_or(0);
            buffer.iter_mut().take(count).for_each(|byte| *byte = 1);
            Ok(count)
        }
    }

    struct BytesAfterEof {
        calls: usize,
    }

    impl AssetReader for BytesAfterEof {
        fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, AssetReadError> {
            self.calls += 1;
            if self.calls == 1 {
                return Ok(0);
            }
            buffer[0] = 1;
            Ok(1)
        }
    }

    fn opened_with_length(bytes: &[u8], length: Option<AssetLength>) -> OpenedAsset {
        OpenedAsset::new(
            AuthorityDomain::new(),
            AssetPath::new("model.bin").expect("valid path"),
            std::io::Cursor::new(bytes.to_vec()),
            length,
        )
    }

    #[test]
    fn cloned_domains_are_stable_and_independent_domains_are_distinct() {
        let first = AuthorityDomain::new();
        let cloned = first.clone();
        let second = AuthorityDomain::new();

        assert_eq!(first, cloned);
        assert_ne!(first, second);

        let domains = HashSet::from([first, cloned, second]);
        assert_eq!(domains.len(), 2);
    }

    #[test]
    fn opened_asset_identity_checks_both_authority_and_path() {
        let domain = AuthorityDomain::new();
        let other_domain = AuthorityDomain::new();
        let path = AssetPath::new("model.bin").expect("valid path");
        let other_path = AssetPath::new("other.bin").expect("valid path");
        let opened = OpenedAsset::new(
            domain.clone(),
            path.clone(),
            std::io::Cursor::new(Vec::<u8>::new()),
            Some(AssetLength::Exact(0)),
        );

        assert!(opened.matches_identity(&domain, &path));
        assert!(!opened.matches_identity(&other_domain, &path));
        assert!(!opened.matches_identity(&domain, &other_path));
    }

    #[test]
    fn exact_length_is_checked_at_both_overrun_and_eof_boundaries() {
        let mut exact = opened_with_length(b"data", Some(AssetLength::Exact(4)));
        let mut chunk = [0_u8; 8];
        assert_eq!(exact.read_chunk(&mut chunk).expect("exact payload"), 4);
        assert_eq!(exact.read_chunk(&mut chunk).expect("exact eof"), 0);

        let mut truncated = opened_with_length(b"abc", Some(AssetLength::Exact(4)));
        assert_eq!(truncated.read_chunk(&mut chunk).expect("payload"), 3);
        assert_eq!(
            truncated
                .read_chunk(&mut chunk)
                .expect_err("premature eof must fail")
                .kind(),
            crate::AssetReadErrorKind::ContractViolation(
                AssetReadContractViolation::ExactLengthTruncated {
                    expected: 4,
                    observed: 3,
                }
            )
        );

        let mut overlong = opened_with_length(b"abcde", Some(AssetLength::Exact(4)));
        assert_eq!(
            overlong
                .read_chunk(&mut chunk)
                .expect_err("overlong exact payload must fail")
                .kind(),
            crate::AssetReadErrorKind::ContractViolation(
                AssetReadContractViolation::ExactLengthExceeded {
                    expected: 4,
                    observed: 5,
                }
            )
        );
    }

    #[test]
    fn exact_truncation_is_latched_without_reentering_the_reader() {
        let (reader, calls) = CountingReader::new(vec![0, 1]);
        let mut asset = OpenedAsset::new(
            AuthorityDomain::new(),
            AssetPath::new("model.bin").expect("valid path"),
            reader,
            Some(AssetLength::Exact(1)),
        );
        let mut chunk = [0_u8; 1];
        let expected = crate::AssetReadErrorKind::ContractViolation(
            AssetReadContractViolation::ExactLengthTruncated {
                expected: 1,
                observed: 0,
            },
        );

        assert_eq!(asset.read_chunk(&mut chunk).unwrap_err().kind(), expected);
        let mut empty = [];
        assert_eq!(asset.read_chunk(&mut empty).expect("empty read"), 0);
        assert_eq!(asset.read_chunk(&mut chunk).unwrap_err().kind(), expected);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(asset.bytes_read(), 0);
    }

    #[test]
    fn exact_overrun_is_latched_and_records_all_observed_bytes() {
        let (reader, calls) = CountingReader::new(vec![5, 0]);
        let mut asset = OpenedAsset::new(
            AuthorityDomain::new(),
            AssetPath::new("model.bin").expect("valid path"),
            reader,
            Some(AssetLength::Exact(4)),
        );
        let mut chunk = [0_u8; 8];
        let expected = crate::AssetReadErrorKind::ContractViolation(
            AssetReadContractViolation::ExactLengthExceeded {
                expected: 4,
                observed: 5,
            },
        );

        assert_eq!(asset.read_chunk(&mut chunk).unwrap_err().kind(), expected);
        assert_eq!(asset.bytes_read(), 5);
        assert_eq!(asset.read_chunk(&mut chunk).unwrap_err().kind(), expected);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(asset.bytes_read(), 5);
    }

    #[test]
    fn length_hint_and_unknown_length_do_not_claim_an_upper_bound_or_exact_eof() {
        for length in [Some(AssetLength::Hint(2)), None] {
            let mut asset = opened_with_length(b"abcde", length);
            let mut chunk = [0_u8; 8];

            assert_eq!(
                asset.read_chunk(&mut chunk).expect("length may be unknown"),
                5
            );
            assert_eq!(asset.bytes_read(), 5);
            assert_eq!(asset.read_chunk(&mut chunk).expect("eof"), 0);
            assert_eq!(asset.bytes_read(), 5);
        }
    }

    #[test]
    fn empty_read_does_not_validate_eof_or_advance_exact_length() {
        let mut asset = opened_with_length(b"data", Some(AssetLength::Exact(4)));
        let mut empty = [];
        let mut chunk = [0_u8; 4];

        assert_eq!(asset.read_chunk(&mut empty).expect("empty read"), 0);
        assert_eq!(asset.read_chunk(&mut chunk).expect("payload"), 4);
        assert_eq!(asset.read_chunk(&mut chunk).expect("verified eof"), 0);
    }

    #[test]
    fn oversized_reader_count_becomes_a_typed_contract_failure() {
        let (reader, calls) = CountingReader::new(vec![5, 0]);
        let mut asset = OpenedAsset::new(
            AuthorityDomain::new(),
            AssetPath::new("model.bin").expect("valid path"),
            reader,
            None,
        );
        let mut chunk = [0_u8; 4];
        let expected = crate::AssetReadErrorKind::ContractViolation(
            AssetReadContractViolation::CountExceedsBuffer {
                returned: 5,
                capacity: 4,
            },
        );

        assert_eq!(asset.read_chunk(&mut chunk).unwrap_err().kind(), expected);
        assert_eq!(asset.read_chunk(&mut chunk).unwrap_err().kind(), expected);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(asset.bytes_read(), 0);
    }

    #[test]
    fn byte_count_overflow_is_permanently_latched() {
        let (reader, calls) = CountingReader::new(vec![1, 1]);
        let mut asset = OpenedAsset::new(
            AuthorityDomain::new(),
            AssetPath::new("model.bin").expect("valid path"),
            reader,
            None,
        );
        asset.bytes_read = u64::MAX;
        let mut chunk = [0_u8; 1];
        let expected = crate::AssetReadErrorKind::ContractViolation(
            AssetReadContractViolation::ByteCountOverflow {
                consumed: u64::MAX,
                returned: 1,
            },
        );

        assert_eq!(asset.read_chunk(&mut chunk).unwrap_err().kind(), expected);
        assert_eq!(asset.read_chunk(&mut chunk).unwrap_err().kind(), expected);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(asset.bytes_read(), u64::MAX);
    }

    #[test]
    fn eof_is_terminal_even_if_a_buggy_reader_would_later_return_bytes() {
        let mut asset = OpenedAsset::new(
            AuthorityDomain::new(),
            AssetPath::new("model.bin").expect("valid path"),
            BytesAfterEof { calls: 0 },
            None,
        );
        let mut chunk = [0_u8; 1];

        assert_eq!(asset.read_chunk(&mut chunk).expect("first eof"), 0);
        assert_eq!(asset.read_chunk(&mut chunk).expect("latched eof"), 0);
        assert_eq!(chunk, [0]);
    }

    #[test]
    fn authority_types_are_safe_to_reuse_across_caught_unwinds() {
        fn assert_ref_unwind_safe<T: RefUnwindSafe>() {}

        assert_ref_unwind_safe::<AuthorityDomain>();
        assert_ref_unwind_safe::<crate::MemoryAssetIo>();
    }
}
