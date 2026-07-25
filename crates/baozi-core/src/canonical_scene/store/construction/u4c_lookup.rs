use baozi_operation::__u4c::{MeasurementReport, MeasurementSession};
use baozi_operation::{
    PhysicalStorageAccount, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, PreparedStoreSuccessOutcome, PublishedCanonicalStore,
    PublishedOutcomeEvidenceStore, PublishedStoreSuccessParts, RecordBlock, RecordPlan,
    RetainedStoreBlockTablePlan, RetainedStorePublicationPlan, StoreLedgerEvidence,
    StoreSuccessValidation,
};

use super::{
    MAX_ADMITTED_CANONICAL_NEW_BLOCKS, SceneConstructionLimits, SceneConstructionOperation,
    SceneStoreKeys, final_progress, scene_writer_plan,
};

#[derive(Clone, Copy)]
struct U4cLookupNodeSchema;

#[derive(Clone, Copy)]
struct U4cLookupEvidenceSchema;

#[derive(Clone, Copy)]
struct U4cLookupForgedIdentitySchema;

struct PublishedU4cLookupFixture {
    canonical: PublishedCanonicalStore<RecordBlock, U4cLookupNodeSchema>,
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, U4cLookupEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct U4cLookupScaleObservation {
    members: usize,
    report: MeasurementReport,
}

const U4C_LOOKUP_EVIDENCE_SENTINEL: u32 = 0x5534_4301;
const MIN_U4C_LOOKUP_MEMBERS: usize = 2;

fn observe_u4c_lookup_scale(members: usize) -> U4cLookupScaleObservation {
    assert!(
        (MIN_U4C_LOOKUP_MEMBERS..=MAX_ADMITTED_CANONICAL_NEW_BLOCKS).contains(&members),
        "the U4c lookup fixture must fit the real canonical writer limits"
    );

    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
            .expect("U4c lookup factory limits must be valid"),
    )
    .expect("U4c lookup physical account factory must start");
    let account = factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, 256)
                .expect("U4c lookup account limits must be valid"),
        )
        .expect("U4c lookup physical account must start");
    let foreign = publish_u4c_lookup_fixture(&account, MIN_U4C_LOOKUP_MEMBERS);

    let session = MeasurementSession::begin().expect("U4c lookup measurement session must start");
    let published = publish_u4c_lookup_fixture(&account, members);

    {
        let root = published.canonical.root();
        let root_index = u32::try_from(members - 1).expect("U4c lookup root index must fit u32");
        assert_eq!(root.scalar::<u32>(0, 0), Some(root_index));
        let child = root
            .record_ref::<U4cLookupNodeSchema>(0, 1)
            .expect("U4c lookup root must point to the preceding high-index member");
        assert_eq!(
            published
                .canonical
                .records(published.canonical.root_ref())
                .and_then(|view| view.scalar::<u32>(0, 0)),
            Some(root_index)
        );
        assert_eq!(
            published
                .canonical
                .records(child)
                .and_then(|view| view.scalar::<u32>(0, 0)),
            Some(root_index - 1)
        );
        assert!(
            published
                .canonical
                .records(foreign.canonical.root_ref())
                .is_none(),
            "a same-kind slot from a foreign retained set must reject"
        );
        let out_of_range = published
            .canonical
            .root_ref()
            .__u4c_probe_with_identity_index(members);
        assert!(
            published.canonical.records(out_of_range).is_none(),
            "a same-set slot outside the retained member directory must reject"
        );
        let tag_mismatch = published
            .canonical
            .root_ref()
            .__u4c_probe_with_identity_tag::<U4cLookupForgedIdentitySchema>();
        assert!(
            published.canonical.records(tag_mismatch).is_none(),
            "a same-set slot with a mismatched identity tag must reject"
        );
    }
    {
        let evidence = published.outcome_evidence.root();
        assert_eq!(
            evidence.scalar::<u32>(0, 0),
            Some(U4C_LOOKUP_EVIDENCE_SENTINEL)
        );
        assert_eq!(
            published
                .outcome_evidence
                .records(published.outcome_evidence.root_ref())
                .and_then(|view| view.scalar::<u32>(0, 0)),
            Some(U4C_LOOKUP_EVIDENCE_SENTINEL)
        );
    }

    let report = session
        .finish()
        .expect("U4c lookup measurement must finish");
    drop_published_fixture(published);
    drop_published_fixture(foreign);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);

    U4cLookupScaleObservation { members, report }
}

fn publish_u4c_lookup_fixture(
    account: &PhysicalStorageAccount,
    members: usize,
) -> PublishedU4cLookupFixture {
    let operation = SceneConstructionOperation::with_account(account, u4c_lookup_limits())
        .expect("U4c lookup core construction operation must start");
    let SceneConstructionOperation { raw, keys, .. } = operation;
    let mut assembly = raw
        .try_publication_assembly(u4c_lookup_publication_plan(keys, members))
        .expect("U4c lookup publication assembly must start");

    let mut prior = None;
    for index in 0..members {
        let scalar = u32::try_from(index).expect("U4c lookup member index must fit u32");
        let prior_for_row = prior;
        let fields = if prior_for_row.is_some() { 2 } else { 1 };
        let node = assembly
            .try_retain_canonical_records(
                RecordPlan::new(1, fields).expect("U4c lookup canonical record plan must be valid"),
                U4cLookupNodeSchema,
                |writer| {
                    writer.write_row(|row| {
                        row.write_scalar(scalar)?;
                        if let Some(prior) = prior_for_row {
                            row.write_record_ref(prior)?;
                        }
                        Ok(())
                    })
                },
            )
            .expect("U4c lookup canonical record must retain");
        prior = Some(node);
    }
    let canonical_root = prior.expect("U4c lookup fixture has at least one canonical record");
    let outcome_evidence_root = assembly
        .try_retain_outcome_evidence_records(
            RecordPlan::new(1, 1).expect("U4c lookup evidence record plan must be valid"),
            U4cLookupEvidenceSchema,
            |writer| writer.write_row(|row| row.write_scalar(U4C_LOOKUP_EVIDENCE_SENTINEL)),
        )
        .expect("U4c lookup outcome-evidence record must retain");

    let validated = match assembly.validate_success(
        canonical_root,
        outcome_evidence_root,
        |validation, canonical_root, outcome_evidence_root| {
            let mut current = canonical_root;
            for index in (0..members).rev() {
                let Some(view) = validation.canonical_records(current) else {
                    return false;
                };
                let expected = u32::try_from(index).expect("U4c lookup member index must fit u32");
                if view.scalar::<u32>(0, 0) != Some(expected) {
                    return false;
                }
                if index != 0 {
                    let Some(previous) = view.record_ref::<U4cLookupNodeSchema>(0, 1) else {
                        return false;
                    };
                    current = previous;
                }
            }

            validation
                .outcome_evidence_records(outcome_evidence_root)
                .and_then(|view| view.scalar::<u32>(0, 0))
                == Some(U4C_LOOKUP_EVIDENCE_SENTINEL)
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("the U4c lookup fixture must validate through the core construction seam"),
    };
    let prepared = match validated
        .try_prepare(final_progress())
        .expect("the U4c lookup fixture must prepare")
    {
        PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("the U4c lookup fixture must remain a successful publication"),
    };
    let PublishedStoreSuccessParts {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = prepared.handoff();
    PublishedU4cLookupFixture {
        canonical,
        outcome_evidence,
        ledger_evidence,
    }
}

fn drop_published_fixture(published: PublishedU4cLookupFixture) {
    let PublishedU4cLookupFixture {
        canonical,
        outcome_evidence,
        ledger_evidence,
    } = published;
    drop(canonical);
    drop(outcome_evidence);
    drop(ledger_evidence);
}

fn u4c_lookup_limits() -> SceneConstructionLimits {
    SceneConstructionLimits {
        operation_live_bytes: u64::MAX,
        dimension_live_bytes: u64::MAX,
        attempted_work: u64::MAX,
        consumed_work: u64::MAX,
        committed_output: u64::MAX,
        published_evidence: u64::MAX,
    }
}

fn u4c_lookup_publication_plan(
    keys: SceneStoreKeys,
    members: usize,
) -> RetainedStorePublicationPlan {
    let canonical_attributions = members
        .checked_add(1)
        .expect("U4c lookup canonical table attribution count must fit");
    let canonical_writer = scene_writer_plan(keys, canonical_attributions, 4)
        .expect("U4c lookup canonical writer plan must be valid");
    let evidence_writer = scene_writer_plan(keys, 2, 4)
        .expect("U4c lookup outcome-evidence writer plan must be valid");
    RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(canonical_writer, members, members)
            .expect("U4c lookup canonical table plan must be valid"),
        RetainedStoreBlockTablePlan::new(evidence_writer, 1, 1)
            .expect("U4c lookup outcome-evidence table plan must be valid"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_MEMBERS: usize = 4;

    #[test]
    #[ignore = "U4c lookup measurement runs as an exact single-test subprocess"]
    fn one_canonical_table_has_constant_lookup_cost_at_n_2n_and_4n() {
        let at_n = observe_u4c_lookup_scale(BASE_MEMBERS);
        let at_2n = observe_u4c_lookup_scale(BASE_MEMBERS * 2);
        let at_4n = observe_u4c_lookup_scale(BASE_MEMBERS * 4);

        assert_observation(&at_n, BASE_MEMBERS);
        assert_observation(&at_2n, BASE_MEMBERS * 2);
        assert_observation(&at_4n, BASE_MEMBERS * 4);
        assert_scale(&at_n, &at_2n, 2);
        assert_scale(&at_n, &at_4n, 4);
    }

    fn assert_observation(observation: &U4cLookupScaleObservation, expected_members: usize) {
        assert_eq!(observation.members, expected_members);
        let members =
            u64::try_from(expected_members).expect("U4c lookup member count must fit u64");
        let blocks = members
            .checked_add(1)
            .expect("U4c lookup block count must fit u64");
        let references = members
            .checked_sub(1)
            .expect("U4c lookup fixture always has at least two members");
        let producer_events = blocks
            .checked_mul(4)
            .and_then(|value| value.checked_add(references))
            .expect("U4c lookup producer count must fit u64");

        assert_eq!(observation.report.semantic_cell_writes(), blocks);
        assert_eq!(observation.report.reference_writes(), references);
        assert_eq!(observation.report.payload_backing_allocations(), blocks);
        assert_eq!(observation.report.block_begins(), blocks);
        assert_eq!(observation.report.block_seals(), blocks);
        assert_eq!(observation.report.slot_publications(), blocks);
        assert_eq!(observation.report.new_block_begins(), blocks);
        assert_eq!(observation.report.new_block_seals(), blocks);
        assert_eq!(observation.report.new_slot_publications(), blocks);
        assert_eq!(observation.report.inherited_block_begins(), 0);
        assert_eq!(observation.report.inherited_block_seals(), 0);
        assert_eq!(observation.report.inherited_slot_publications(), 0);
        assert_eq!(observation.report.producer_events(), producer_events);
        assert_eq!(observation.report.element_decodes(), 0);
        assert_eq!(observation.report.completed_lookups(), 8);
        assert_eq!(observation.report.rejected_lookups(), 3);
        assert_eq!(observation.report.lookup_events(), 51);
        assert_eq!(observation.report.max_lookup_events(), 8);
        // Five successful lookups and two same-set rejections reach the member directory.
        // The foreign-set rejection stops before probing, and every reached lookup probes once.
        assert_eq!(observation.report.member_probes(), 7);
        assert_eq!(observation.report.max_member_probes_per_lookup(), 1);
        assert!(observation.report.payload_copy_bytes() > 0);
    }

    fn assert_scale(
        baseline: &U4cLookupScaleObservation,
        scaled: &U4cLookupScaleObservation,
        multiplier: u64,
    ) {
        let expected_members = u64::try_from(baseline.members)
            .expect("U4c lookup baseline member count must fit u64")
            .checked_mul(multiplier)
            .expect("U4c lookup scaled member count must fit u64");
        assert_eq!(
            u64::try_from(scaled.members).expect("U4c lookup scaled member count must fit u64"),
            expected_members
        );
        assert_eq!(
            scaled.report.payload_copy_bytes(),
            baseline
                .report
                .payload_copy_bytes()
                .checked_mul(multiplier)
                .expect("U4c lookup scaled payload copy budget must fit u64"),
            "record payload copy work must scale exactly with reachable members"
        );
        assert_eq!(
            scaled.report.completed_lookups(),
            baseline.report.completed_lookups(),
            "the measured public lookup count must not grow with retained member count"
        );
        assert_eq!(
            scaled.report.rejected_lookups(),
            baseline.report.rejected_lookups(),
            "the measured rejection count must not grow with retained member count"
        );
        assert_eq!(
            scaled.report.lookup_events(),
            baseline.report.lookup_events(),
            "the taxonomy work for the fixed public lookups must remain constant"
        );
        assert_eq!(
            scaled.report.max_lookup_events(),
            baseline.report.max_lookup_events(),
            "the maximum taxonomy work of one public lookup must remain constant"
        );
        assert_eq!(
            scaled.report.member_probes(),
            baseline.report.member_probes(),
            "the actual indexed member probes must remain constant"
        );
        assert_eq!(
            scaled.report.max_member_probes_per_lookup(),
            baseline.report.max_member_probes_per_lookup(),
            "one lookup must never probe more retained members at larger table sizes"
        );
    }
}
