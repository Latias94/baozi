use baozi_operation::PhysicalStorageAccount;

use super::{successful_parts, test_account, test_factory, test_limits};
use crate::canonical_scene::store::construction::node_scene::NodeSceneTestTransaction;
use crate::canonical_scene::store::production::*;
use crate::extension::{ExtensionMediaType, ExtensionValueRef};
use crate::provenance::{
    ContentDigest, EvidenceDigest, OperationEvidenceSummary, OperationKind, OptionsDigest,
    ResourceIdentity, SourceObjectLocator,
};
use crate::{
    DigestAlgorithm, FormatId, MetadataOwnerRef, MetadataValueRef, NamespacedName,
    ProvenanceKindRef, ProvenanceOperationKindRef, SceneSpace, SceneValidationLimits,
    SourceObjectKind, SourceObjectLocatorRef, ValidatedScene,
};

const JSON: &[u8] = br#" { "exact": [null, 1e+02, "\u0061"] } "#;

fn provenance_scene(account: &PhysicalStorageAccount) -> ValidatedScene {
    let resource = ResourceIdentity::new("fixtures/provenance.scene").unwrap();
    let resources = [ObservedResourceAuthoring::session_opaque(
        &resource,
        SessionOpaqueReason::CallerLabel,
    )];
    let format = FormatId::new("org.baozi.provenance-test").unwrap();
    let locators = [
        SourceObjectLocator::indexed(SourceObjectKind::Mesh, 93),
        SourceObjectLocator::indexed(SourceObjectKind::Mesh, 17),
    ];
    let sources = locators.each_ref().map(|locator| {
        SourceIdentityAuthoring::new(ResourceIdentityLocalId::new(0), &format, locator)
            .with_content_digest(ContentDigest::sha256([11; 32]))
    });
    let generated = OperationKind::Generated;
    let operation = OperationKind::Namespaced(NamespacedName::new("test:merge").unwrap());
    let parents = [ProvenanceLocalId::new(0), ProvenanceLocalId::new(1)];
    let evidence = [
        OperationEvidenceSummary::new(
            NamespacedName::new("test:first").unwrap(),
            2,
            Some(EvidenceDigest::blake3([13; 32])),
        )
        .unwrap(),
        OperationEvidenceSummary::new(NamespacedName::new("test:second").unwrap(), 4, None)
            .unwrap(),
    ];
    let provenance = [
        ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(1)),
        ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(0)),
        ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
            &generated,
            OptionsDigest::sha256([5; 32]),
            &[],
            &[],
        )),
        ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
            &operation,
            OptionsDigest::blake3([7; 32]),
            &parents,
            &evidence,
        )),
    ];
    let attachments = [ProvenanceLocalId::new(1), ProvenanceLocalId::new(3)];
    let generated_attachment = [ProvenanceLocalId::new(2)];
    let owners = [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &attachments),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &generated_attachment),
    ];
    let key = NamespacedName::new("test:extras").unwrap();
    let media_type = ExtensionMediaType::new("application/json").unwrap();
    let metadata = [MetadataAuthoringEntry::for_source(
        SourceIdentityLocalId::new(0),
        &key,
        ExtensionValueRef::Bytes {
            media_type: &media_type,
            payload: JSON,
        },
    )];
    let parts = successful_parts(
        NodeSceneTestTransaction::with_account(
            account,
            test_limits(),
            SceneAuthoringBatch::new(
                SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
                NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
                AppearanceAuthoringBatch::empty(),
                GeometryAuthoringBatch::empty(),
                DeformationAuthoringBatch::empty(),
                AnimationAuthoringBatch::empty(),
                ProfileAuthoringBatch::new(
                    LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
                    MetadataAuthoringBatch::new(&metadata),
                ),
            ),
            SceneValidationLimits::default(),
        )
        .unwrap()
        .finish_draft()
        .unwrap()
        .publish_for_test()
        .unwrap(),
    );
    ValidatedScene::from_node_scene(parts.scene)
}

#[test]
fn retained_relations_summaries_and_source_metadata_reads_allocate_nothing() {
    let factory = test_factory();
    let account = test_account(&factory, 1024);
    let scene = provenance_scene(&account);
    let allocations = allocation_counter::measure(|| {
        assert_eq!(scene.provenance_node_count(), 4);
        let mut all = scene.provenance_nodes();
        let first = all.next().unwrap();
        let last = all.next_back().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(last.depth(), 2);
        assert_eq!(last.id().generation(), scene.generation());
        assert_eq!(scene.provenance(last.id()).unwrap(), last);
        assert_eq!(last.parents().len(), 2);
        let mut parents = last.parents();
        assert_eq!(parents.clone().next().unwrap(), first);
        assert_eq!(parents.next().unwrap(), first);
        let other_parent = parents.next_back().unwrap();
        assert_eq!(parents.len(), 0);
        assert!(parents.next().is_none());
        assert_eq!(first.depth(), 1);
        assert_eq!(first.parents().len(), 0);
        let ProvenanceKindRef::Source(empty_source) = first.kind() else {
            panic!("first retained node is a source")
        };
        assert_eq!(empty_source.index(), 1);
        assert_eq!(empty_source.metadata_entries().len(), 0);
        let ProvenanceKindRef::Source(source) = other_parent.kind() else {
            panic!("second retained parent is a source")
        };
        assert_eq!(source.index(), 0);
        assert_eq!(source.generation(), scene.generation());
        assert_eq!(source.resource().identity(), "fixtures/provenance.scene");
        assert_eq!(
            source.locator(),
            SourceObjectLocatorRef::Indexed {
                kind: SourceObjectKind::Mesh,
                index: 93,
            }
        );
        assert_eq!(
            source.content_digest(),
            Some((DigestAlgorithm::Sha256, [11; 32].as_slice()))
        );
        let entry = source.metadata_entries().next_back().unwrap();
        assert_eq!(entry.owner(), MetadataOwnerRef::SourceObject(source));
        assert_eq!(entry.key(), "test:extras");
        let MetadataValueRef::Bytes {
            media_type,
            payload,
        } = entry.value()
        else {
            panic!("source extras are retained bytes")
        };
        assert_eq!((media_type, payload), ("application/json", JSON));
        let MetadataValueRef::Bytes { payload: again, .. } =
            scene.metadata_entries().next().unwrap().value()
        else {
            panic!("inventory and source view borrow the same entry")
        };
        assert!(std::ptr::eq(payload, again));
        let ProvenanceKindRef::Operation(operation) = last.kind() else {
            panic!("last retained node is an operation")
        };
        assert_eq!(
            operation.kind(),
            ProvenanceOperationKindRef::Namespaced("test:merge")
        );
        assert_eq!(
            operation.options_digest(),
            (DigestAlgorithm::Blake3, [7; 32].as_slice())
        );
        assert_eq!(operation.evidence_count(), 2);
        let mut evidence = operation.evidence();
        assert_eq!(evidence.len(), 2);
        let first_evidence = evidence.next().unwrap();
        assert_eq!(first_evidence.code(), "test:first");
        assert_eq!(first_evidence.occurrence_count(), 2);
        assert_eq!(
            first_evidence.digest(),
            Some((DigestAlgorithm::Blake3, [13; 32].as_slice()))
        );
        assert!(std::ptr::eq(
            first_evidence.digest().unwrap().1,
            operation.evidence().next().unwrap().digest().unwrap().1,
        ));
        let second_evidence = evidence.next_back().unwrap();
        assert_eq!(second_evidence.code(), "test:second");
        assert_eq!(second_evidence.occurrence_count(), 4);
        assert_eq!(second_evidence.digest(), None);
        assert_eq!(evidence.len(), 0);
        let ProvenanceKindRef::Operation(generated) =
            scene.provenance_nodes().nth(2).unwrap().kind()
        else {
            panic!("third retained node is generated")
        };
        assert_eq!(generated.kind(), ProvenanceOperationKindRef::Generated);
        assert_eq!(generated.evidence_count(), 0);
        assert_eq!(generated.evidence().len(), 0);
        assert_eq!(scene.provenance_nodes().nth(2).unwrap().parents().len(), 0);
        let mut attachments = scene.owner_provenance(scene.metadata_owner()).unwrap();
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments.clone().next().unwrap(), other_parent);
        assert_eq!(attachments.next_back().unwrap(), last);
        assert_eq!(attachments.next().unwrap(), other_parent);
        assert!(attachments.next().is_none());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
}
