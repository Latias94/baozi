use super::*;

#[test]
fn final_owner_source_uses_only_the_qualified_extraction_path() {
    let non_final_owner_production = retained_non_final_owner_production_source();
    let final_owner = include_str!("../final_owner.rs");
    let production = retained_production_source();

    for extraction in [
        "Arc::into_unique",
        "Arc::try_unique",
        "UniqueArc::into_inner",
    ] {
        assert!(
            !non_final_owner_production.contains(extraction),
            "final-owner extraction `{extraction}` escaped final_owner.rs"
        );
        assert!(
            final_owner.contains(extraction),
            "final_owner.rs lost required extraction `{extraction}`"
        );
    }
    assert_eq!(
        final_owner.matches("Arc::into_unique(shared)").count(),
        2,
        "ordinary release and final shared-owner drop are the only non-strict transitions"
    );
    assert_eq!(
        final_owner.matches("Arc::try_unique(shared)").count(),
        1,
        "strict pre-attribution cleanup has one qualified uniqueness transition"
    );
    assert_eq!(
        final_owner.matches("UniqueArc::into_inner(unique)").count(),
        3,
        "every qualified uniqueness transition extracts through the generic engine"
    );
    for forbidden in [
        "Arc::try_unwrap",
        "Arc::into_raw",
        "Arc::from_raw",
        "ArcBorrow",
        "ArcUnion",
        "OffsetArc",
        "ThinArc",
        "impl<T> Clone for RetainedAllocation",
        "impl Clone for PhysicalStorageAccount {",
    ] {
        assert!(
            !production.contains(forbidden),
            "forbidden final-owner bypass `{forbidden}` was introduced"
        );
    }
}

#[test]
fn terminal_publication_protocol_stays_out_of_retained_root() {
    let retained_root = retained_root_production_source();
    let terminal = include_str!("../terminal.rs");

    for (definition, root_forbidden) in [
        (
            "pub(super) struct TerminalPhysicalOwnershipManifest",
            "struct TerminalPhysicalOwnershipManifest",
        ),
        (
            "pub(super) struct TerminalSuccessFreezeGuard",
            "struct TerminalSuccessFreezeGuard",
        ),
        (
            "pub(super) struct PrimaryPublicationBatches",
            "struct PrimaryPublicationBatches",
        ),
        (
            "pub(super) struct TerminalPrimaryGuard",
            "struct TerminalPrimaryGuard",
        ),
        (
            "pub(super) struct TerminalFallbackGuard",
            "struct TerminalFallbackGuard",
        ),
        (
            "pub(crate) enum RetainedPublicationPrepareError",
            "enum RetainedPublicationPrepareError",
        ),
        (
            "pub(super) struct QualifiedDetachedTerminalEvidence",
            "struct QualifiedDetachedTerminalEvidence",
        ),
    ] {
        assert!(
            terminal.contains(definition),
            "terminal protocol declaration `{definition}` must remain in terminal.rs"
        );
        assert!(
            !retained_root.contains(root_forbidden),
            "terminal protocol declaration `{root_forbidden}` leaked back into retained.rs"
        );
    }
}

#[test]
fn generic_retained_authority_remains_crate_private() {
    let production = retained_production_source();
    let root_exports = include_str!("../../lib.rs");
    let test_attribute = ["#[cfg(", "test)]"].concat();

    assert!(
        !root_exports.contains("pub use retained::*"),
        "the private retained module must never gain a glob re-export"
    );
    let export_prefix = "pub use retained::{";
    assert_eq!(root_exports.matches(export_prefix).count(), 1);
    let export_tail = root_exports
        .split_once(export_prefix)
        .expect("the crate root has one curated retained export block")
        .1;
    let export_body = export_tail
        .split_once("};")
        .expect("the curated retained export block is terminated")
        .0;
    let mut actual_exports: Vec<_> = export_body
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    let mut expected_exports = vec![
        "AccountPoisonEvidence",
        "AccountPoisonReason",
        "AccountPoisoned",
        "FinalizationContainmentError",
        "PhysicalStorageAccount",
        "PhysicalStorageAccountCreateError",
        "PhysicalStorageAccountFactory",
        "PhysicalStorageAccountFactoryCreateError",
        "PhysicalStorageAccountFactoryLimits",
        "PhysicalStorageAccountFactorySnapshot",
        "PhysicalStorageAccountLimits",
        "PhysicalStorageAccountSnapshot",
        "PhysicalStorageLimitError",
    ];
    actual_exports.sort_unstable();
    expected_exports.sort_unstable();
    assert_eq!(actual_exports, expected_exports);

    let mut public_declarations: Vec<_> = production
        .lines()
        .filter(|line| {
            line.starts_with("pub struct ")
                || line.starts_with("pub enum ")
                || line.starts_with("pub const ")
                || line.starts_with("pub type ")
                || line.starts_with("pub trait ")
        })
        .filter_map(|line| line.split_whitespace().nth(2))
        .map(|name| name.trim_end_matches(['{', ':']))
        .collect();
    public_declarations.sort_unstable();
    assert_eq!(public_declarations, expected_exports);

    assert!(
        production.contains(
            "type RetainedAllocation<T> = PolicyRetainedAllocation<T, LeafRetainedPolicy>;"
        )
    );
    assert!(!production.contains("pub(crate) type RetainedAllocation"));
    assert!(production.contains("\npub(super) struct RetainedPart<"));
    assert!(!production.contains("\npub(crate) struct RetainedPart<"));
    assert!(production.contains("\npub(super) enum RetainedClosureValidationError"));
    for forbidden in [
        "RetainedAllocation",
        "RetainedPart",
        "RetainedOperation",
        "RetainedAllocationPlan",
        "BlockEmplacer",
        "RetainedClosureValidationError",
        "RetainedSuccessClosureProof",
        "RetainedEvidenceClosureProof",
        "PublishedRetainedAggregate",
        "FinalOwnerAllocation",
        "AdmissionLease",
    ] {
        assert!(
            !actual_exports.contains(&forbidden),
            "raw retained authority `{forbidden}` escaped the crate root"
        );
    }

    {
        let method = "try_retain_with_admitted_handle";
        let needle = format!("fn {method}");
        let signatures: Vec<_> = production
            .lines()
            .filter(|line| line.contains(&needle))
            .collect();
        assert_eq!(
            signatures.len(),
            1,
            "raw physical mutator `{method}` must have one implementation"
        );
        assert!(
            matches!(
                signatures[0].trim_start(),
                signature if signature.starts_with(&needle)
                    || signature.starts_with(&format!("pub(super) {needle}"))
            ),
            "raw physical mutator `{method}` escaped the retained module: {}",
            signatures[0]
        );
    }
    assert!(
        !production.contains("fn share_after_admission"),
        "raw account sharing must not exist in production"
    );
    let count_test_only_wrappers = |candidate: &str| {
        candidate
            .lines()
            .zip(candidate.lines().skip(1))
            .filter(|(attribute, signature)| {
                attribute.trim_start() == test_attribute.as_str()
                    && *signature == "    pub(super) fn try_retain_with<T>("
            })
            .count()
    };
    assert_eq!(
        count_test_only_wrappers(&production),
        1,
        "unadmitted physical retain wrapper must remain test-only"
    );
    let crlf_production = production.replace("\r\n", "\n").replace('\n', "\r\n");
    assert_eq!(
        count_test_only_wrappers(&crlf_production),
        1,
        "unadmitted physical retain wrapper gate must be line-ending independent"
    );
    for forbidden in [
        "fn try_prepare_success(",
        "fn try_prepare_rich_failure(",
        "pub struct RetainedSuccessClosureProof",
        "pub struct RetainedEvidenceClosureProof",
        "pub fn attach(",
        "pub fn detach(",
        "pub fn get_mut(",
        "pub fn into_block(",
        "pub fn replace(",
        "pub fn retag(",
        "pub fn retire(",
        "fn reset_poison(",
        "fn clear_poison(",
        "fn unpoison(",
        "fn rebrand_account(",
        "fn bypass_account_poison(",
        "RetainedAllocationVisitor",
    ] {
        assert!(
            !production.contains(forbidden),
            "raw retained authority `{forbidden}` escaped the deep module"
        );
    }
}

#[test]
fn retained_handles_do_not_gain_unattributed_clone() {
    trait AmbiguousIfClone<Marker> {
        fn assert_not_clone() {}
    }

    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    impl<T: Clone> AmbiguousIfClone<u8> for T {}

    let _ = <PhysicalStorageAccount as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <RetainedAllocation<u64> as AmbiguousIfClone<_>>::assert_not_clone;
}

#[test]
fn public_handles_are_send_and_sync_for_thread_safe_payloads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PhysicalStorageAccountFactory>();
    assert_send_sync::<PhysicalStorageAccount>();
    assert_send_sync::<RetainedAllocation<u64>>();
}
