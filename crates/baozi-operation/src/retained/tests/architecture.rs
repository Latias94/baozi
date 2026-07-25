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

    let normalized = production.replace("\r\n", "\n");
    assert!(normalized.contains(
        "pub(super) fn try_publication_eligibility(\n        &self,\n    ) -> Result<PublicationEligibility, AccountPoisoned>"
    ));
    assert_eq!(
        normalized.matches(".try_publication_eligibility()").count(),
        3
    );
    for forbidden in [
        "struct TerminalFreezeGuard",
        "struct EligibleTerminalFreezeGuard",
        "struct FallbackFreezeGuard",
        "struct EligibleFallbackFreezeGuard",
        "struct QualifiedTerminalEvidenceFreeze",
        "fn try_freeze_terminal_evidence",
        "struct ExactTerminalPhysicalOwnership",
        "struct ExactFallbackPhysicalOwnership",
        "struct TerminalTableCleanup",
    ] {
        assert!(
            !normalized.contains(forbidden),
            "obsolete terminal rail `{forbidden}` regained production authority"
        );
    }
    for (qualified, eligible) in [
        (
            "struct QualifiedSuccessGraph",
            "struct EligibleSuccessGraph",
        ),
        ("struct QualifiedRichGraph", "struct EligibleRichGraph"),
        (
            "struct QualifiedMinimalGraph",
            "struct EligibleMinimalGraph",
        ),
    ] {
        assert!(
            normalized.contains(qualified),
            "missing owning state `{qualified}`"
        );
        assert!(
            normalized.contains(eligible),
            "missing owning state `{eligible}`"
        );
    }
    for continuation in [
        "struct RichDiscardContinuationState",
        "struct MinimalDiscardContinuationState",
        "contain_rich_discard_continuation_on_drop",
        "contain_minimal_discard_continuation_on_drop",
    ] {
        assert!(
            normalized.contains(continuation),
            "discard continuation lost owning guard `{continuation}`"
        );
    }
    assert_eq!(
        normalized.matches("fn finish_discard(").count(),
        2,
        "rich and minimal discard rails must each have one guarded continuation"
    );
    for guard in [
        "scopeguard::guard(\n            state,\n            contain_rich_discard_continuation_on_drop",
        "scopeguard::guard(\n            state,\n            contain_minimal_discard_continuation_on_drop",
    ] {
        assert!(
            normalized.contains(guard),
            "discard continuation lost its owning scopeguard `{guard}`"
        );
    }
    let minimal_discard = normalized
        .split_once("enum QualifiedMinimalDiscard")
        .expect("qualified minimal discard exists")
        .1
        .split_once("struct QualifiedMinimalState")
        .expect("qualified minimal discard is bounded")
        .0;
    for required in ["    None {", "    Evidence {", "    Both {"] {
        assert!(
            minimal_discard.contains(required),
            "minimal discard lost supported state `{required}`"
        );
    }
    assert!(
        !minimal_discard.contains("    Canonical {"),
        "minimal fallback must reject a canonical-only discard before entering the terminal rail"
    );
    assert!(!normalized.contains("QualifiedMinimalDiscard::Canonical"));
    for continuation_proof in [
        "enum RichDiscardContinuationProof",
        "enum MinimalDiscardContinuationProof",
    ] {
        let variants = normalized
            .split_once(continuation_proof)
            .unwrap_or_else(|| panic!("missing continuation proof `{continuation_proof}`"))
            .1
            .split_once("\n}\n\nimpl")
            .unwrap_or_else(|| panic!("unbounded continuation proof `{continuation_proof}`"))
            .0;
        assert!(variants.contains("    Both("));
        assert!(variants.contains("    Evidence("));
        assert!(!variants.contains("    Canonical("));
        assert!(!variants.contains("    None("));
    }
    for qualification in [
        "proof.try_into_qualified_evidence_discard()",
        "proof.try_into_qualified_discards()",
    ] {
        assert!(
            normalized.contains(qualification),
            "terminal rail lost owning ledger qualification `{qualification}`"
        );
    }

    let success_freeze = normalized
        .split_once(
            "impl<O, P, D, F, CanonicalBlock, EvidenceBlock>\n    FreezeReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>",
        )
        .expect("freeze-ready success owner exists")
        .1
        .split_once("fn contain_freeze_ready_success_state")
        .expect("freeze-ready success owner is bounded")
        .0;
    for required in [
        "let Some(state) = self.state.take()",
        "scopeguard::guard(",
        "let snapshot = state.proof.freeze_in_place();",
        ".freeze_in_place(TerminalEvidenceCause::Success, &snapshot);",
        "scopeguard::ScopeGuard::into_inner(state)",
        "evidence.into_frozen_owner()",
    ] {
        assert!(
            success_freeze.contains(required),
            "success freeze tail lost guarded pure step `{required}`"
        );
    }
    let success_mutation = success_freeze
        .split_once("scopeguard::guard(")
        .expect("success mutation guard exists")
        .1;
    let proof_freeze = success_mutation
        .find("state.proof.freeze_in_place()")
        .expect("typed proof freezes in place");
    let evidence_freeze = success_mutation
        .find(".freeze_in_place(TerminalEvidenceCause::Success, &snapshot)")
        .expect("terminal evidence freezes in place");
    let disarm = success_mutation
        .find("scopeguard::ScopeGuard::into_inner(state)")
        .expect("scopeguard disarms after both freezes");
    assert!(proof_freeze < evidence_freeze);
    assert!(evidence_freeze < disarm);
    let success_handoff = &success_mutation[disarm..];
    for forbidden in [
        "Result<",
        "?",
        "unwrap",
        "expect(",
        "catch_unwind",
        "try_publication_eligibility",
        "try_discard",
        "try_allocate",
        "record_poison",
    ] {
        assert!(
            !success_handoff.contains(forbidden),
            "success handoff regained post-freeze fault gate `{forbidden}`"
        );
    }
    let concrete_evidence = normalized
        .split_once("struct FreezeReadyDetachedTerminalEvidence")
        .expect("concrete terminal evidence owner exists")
        .1
        .split_once("struct FreezeReadySuccessGraph")
        .expect("concrete terminal evidence owner is bounded")
        .0;
    assert!(
        concrete_evidence.contains("owner: DetachedRetainedPart<TerminalLedgerEvidence<P, D>>")
    );
    assert!(concrete_evidence.contains("qualification: TerminalEvidenceFreezeQualification"));

    let qualified_evidence = normalized
        .split_once("struct QualifiedDetachedTerminalEvidence")
        .expect("owning terminal-evidence qualification exists")
        .1
        .split_once("#[cfg(test)]")
        .expect("owning terminal-evidence qualification is bounded")
        .0;
    assert!(
        qualified_evidence
            .contains("owner: Option<DetachedRetainedPart<TerminalLedgerEvidence<P, D>>>")
    );
    assert!(
        qualified_evidence.contains("qualification: Option<TerminalEvidenceFreezeQualification>")
    );
    assert!(qualified_evidence.contains("catch_unwind"));
    assert!(qualified_evidence.contains("owner.allocation().as_ref().try_qualify("));
    assert!(qualified_evidence.contains("owner.record_forgotten_physical_edge();"));
    assert!(!qualified_evidence.contains("&'evidence TerminalLedgerEvidence"));

    let freeze_ready_section = |owner_marker: &str, drop_marker: &str| {
        normalized
            .split_once(owner_marker)
            .unwrap_or_else(|| panic!("missing freeze-ready owner `{owner_marker}`"))
            .1
            .split_once(drop_marker)
            .unwrap_or_else(|| panic!("unbounded freeze-ready owner `{owner_marker}`"))
            .0
    };
    for (owner_marker, drop_marker, cause_marker) in [
        (
            "impl<O, P, D, F, CanonicalBlock, EvidenceBlock>\n    FreezeReadySuccessGraph<O, P, D, F, CanonicalBlock, EvidenceBlock>\nwhere",
            "Drop\n    for FreezeReadySuccessGraph",
            "TerminalEvidenceCause::Success",
        ),
        (
            "impl<O, P, D, F, EvidenceBlock> FreezeReadyRichGraph<O, P, D, F, EvidenceBlock>\nwhere",
            "Drop for FreezeReadyRichGraph",
            "TerminalEvidenceCause::Failure",
        ),
        (
            "impl<O, P, D, F> FreezeReadyMinimalGraph<O, P, D, F>\nwhere",
            "Drop for FreezeReadyMinimalGraph",
            "TerminalEvidenceCause::Failure",
        ),
    ] {
        let freeze = freeze_ready_section(owner_marker, drop_marker);
        for required in [
            "scopeguard::guard(",
            "proof.freeze_in_place()",
            ".freeze_in_place(",
            cause_marker,
            "scopeguard::ScopeGuard::into_inner(state)",
        ] {
            assert!(
                freeze.contains(required),
                "freeze-ready rail `{owner_marker}` lost guarded step `{required}`"
            );
        }
        let proof_freeze = freeze
            .find("proof.freeze_in_place()")
            .expect("ledger proof freezes first");
        let evidence_freeze = freeze[proof_freeze..]
            .find(".freeze_in_place(")
            .map(|position| position + proof_freeze)
            .expect("terminal evidence freezes second");
        let disarm = freeze
            .find("scopeguard::ScopeGuard::into_inner(state)")
            .expect("paired freeze guard disarms last");
        assert!(proof_freeze < evidence_freeze);
        assert!(evidence_freeze < disarm);
        let handoff = &freeze[disarm..];
        for forbidden in [
            "Result<",
            "?",
            "catch_unwind",
            "try_publication_eligibility",
            "try_qualify",
            "try_discard",
            "record_poison",
        ] {
            assert!(
                !handoff.contains(forbidden),
                "freeze-ready handoff `{owner_marker}` regained fault gate `{forbidden}`"
            );
        }
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
