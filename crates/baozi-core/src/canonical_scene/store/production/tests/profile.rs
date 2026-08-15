use std::cell::Cell;
use std::rc::Rc;
use std::sync::OnceLock;

use crate::canonical_scene::store::profile_provenance::issued_profile_segment_nonce_count_for_test;
use crate::extension::{ExtensionArrayRef, ExtensionValueRef};
use crate::math::Vec3;
use crate::namespaced::NamespacedName;
use crate::provenance::{
    ContentDigest, OperationKind, OptionsDigest, ResourceIdentity, SourceDisplayName,
    SourceObjectKind, SourceObjectLocator,
};
use crate::scene_space::SceneSpace;
use crate::{FormatId, SceneValidationLimits};

use super::super::plan::animation::validate_animation_batch_with_control;
use super::super::plan::appearance::validate_appearance_batch_with_control;
use super::super::plan::deformation::validate_deformation_batch_with_control;
use super::super::plan::geometry::validate_geometry_batch_with_control;
use super::super::plan::spatial::validate_spatial_batch_with_control;
use super::super::{
    AnimationAuthoringBatch, AnimationTargetCatalog, AppearanceAuthoringBatch,
    CanonicalProfileOwner, DeformationAuthoringBatch, GeometryAuthoringBatch,
    LineageAuthoringBatch, MetadataAuthoringBatch, MetadataAuthoringEntry, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, ObservedResourceAuthoring, OwnerProvenanceAuthoring, Presence,
    ProfileAuthoringBatch, ProfileAuthoringFailure, ProfileAuthoringLimit,
    ProfileAuthoringSemanticError, ProfileOwner, ProfileOwnerCatalog, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ProvenanceOperationAuthoring, ResourceIdentityLocalId,
    ResourceIdentityStability, SessionOpaqueReason, SourceIdentityAuthoring, SourceIdentityLocalId,
    SpatialAuthoringBatch, StableResourceIdentityFixture, ValidatedAnimationBatch,
    ValidatedAppearanceBatch, ValidatedDeformationBatch, ValidatedGeometryBatch,
    ValidatedNodeBatch, ValidatedProfileBatch, ValidatedSpatialBatch,
    classify_stable_resource_identity_fixture,
};
use super::{TestControl, TestControlError, TestReservation, trs, validate};

const RESOURCE_ID: ResourceIdentityLocalId = ResourceIdentityLocalId::new(0);
const SOURCE_ID: SourceIdentityLocalId = SourceIdentityLocalId::new(0);
const SOURCE_PROVENANCE: ProvenanceLocalId = ProvenanceLocalId::new(0);
const GENERATED_PROVENANCE: ProvenanceLocalId = ProvenanceLocalId::new(1);

struct FixtureVocabulary {
    resource: ResourceIdentity,
    format: FormatId,
    locator: SourceObjectLocator,
    display_name: SourceDisplayName,
    operation_kind: OperationKind,
    metadata_key: NamespacedName,
}

fn vocabulary() -> &'static FixtureVocabulary {
    static VOCABULARY: OnceLock<FixtureVocabulary> = OnceLock::new();
    VOCABULARY.get_or_init(|| FixtureVocabulary {
        resource: ResourceIdentity::new("fixtures/profile/model.glb")
            .expect("test resource identity must be valid"),
        format: FormatId::new("org.baozi.gltf").expect("test format id must be valid"),
        locator: SourceObjectLocator::indexed(SourceObjectKind::Document, 0),
        display_name: SourceDisplayName::new("profile fixture")
            .expect("test display name must be valid"),
        operation_kind: OperationKind::Generated,
        metadata_key: NamespacedName::new("gltf:generator_version")
            .expect("test metadata key must be valid"),
    })
}

fn resource_rows() -> [ObservedResourceAuthoring<'static>; 1] {
    let vocabulary = vocabulary();
    [ObservedResourceAuthoring::session_opaque(
        &vocabulary.resource,
        SessionOpaqueReason::CallerLabel,
    )]
}

fn source_row(resource: ResourceIdentityLocalId) -> SourceIdentityAuthoring<'static> {
    let vocabulary = vocabulary();
    SourceIdentityAuthoring::new(resource, &vocabulary.format, &vocabulary.locator)
        .with_display_name(&vocabulary.display_name)
        .with_content_digest(ContentDigest::sha256([7; 32]))
}

fn mandatory_owner_rows<'input>(
    scene: &'input [ProvenanceLocalId],
    synthetic_root: &'input [ProvenanceLocalId],
    node: &'input [ProvenanceLocalId],
) -> [OwnerProvenanceAuthoring<'input>; 3] {
    [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, scene),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, synthetic_root),
        OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(0)), node),
    ]
}

struct ValidatedDomains<'input> {
    spatial: ValidatedSpatialBatch<'input, TestReservation>,
    nodes: ValidatedNodeBatch<'input, TestReservation>,
    appearance: ValidatedAppearanceBatch<'input, TestReservation>,
    geometry: ValidatedGeometryBatch<'input, TestReservation>,
    deformation: ValidatedDeformationBatch<'input, TestReservation>,
    animation: ValidatedAnimationBatch<'input, TestReservation>,
}

impl ValidatedDomains<'_> {
    fn owners(&self) -> ProfileOwnerCatalog<'_> {
        ProfileOwnerCatalog::from_validated(
            &self.spatial,
            &self.nodes,
            &self.appearance,
            &self.geometry,
            &self.deformation,
            &self.animation,
        )
        .expect("the six validated fixture domains must form one owner catalog")
    }
}

fn validate_profile_batch_with_control<'input>(
    lineage: LineageAuthoringBatch<'input>,
    metadata: MetadataAuthoringBatch<'input>,
    owners: ProfileOwnerCatalog<'_>,
    limits: SceneValidationLimits,
    control: &mut TestControl,
) -> Result<ValidatedProfileBatch<'input, TestReservation>, ProfileAuthoringFailure<TestControlError>>
{
    super::super::plan::profile::validate_profile_batch_with_control(
        ProfileAuthoringBatch::new(lineage, metadata),
        owners,
        limits,
        control,
    )
}

fn validated_domains<'input>(
    roots: &'input [NodeLocalId],
    nodes: &'input [NodeAuthoringNode],
    control: &mut TestControl,
) -> ValidatedDomains<'input> {
    let limits = SceneValidationLimits::default();
    let node_input = NodeAuthoringBatch::new(Presence::Present(roots), nodes);
    let spatial = validate_spatial_batch_with_control(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        node_input,
        limits,
        control,
    )
    .expect("the fixture spatial domain must validate");
    let nodes = validate(Presence::Present(roots), nodes, limits, control)
        .expect("the fixture node domain must validate");
    let appearance =
        validate_appearance_batch_with_control(AppearanceAuthoringBatch::empty(), limits, control)
            .expect("the fixture appearance domain must validate");
    let geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        roots.len(),
        appearance.material_count(),
        limits,
        control,
    )
    .expect("the fixture geometry domain must validate");
    let deformation = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::empty(),
        &nodes,
        &geometry,
        limits,
        control,
    )
    .expect("the fixture deformation domain must validate");
    let targets = AnimationTargetCatalog::new(&nodes, &deformation);
    let animation = validate_animation_batch_with_control(
        AnimationAuthoringBatch::empty(),
        &targets,
        limits,
        control,
    )
    .expect("the fixture animation domain must validate");
    ValidatedDomains {
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animation,
    }
}

fn validate_minimal_profile<'input>(
    lineage: LineageAuthoringBatch<'input>,
    metadata: MetadataAuthoringBatch<'input>,
    control: &mut TestControl,
) -> ValidatedProfileBatch<'input, TestReservation> {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut setup_control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut setup_control);

    validate_profile_batch_with_control(
        lineage,
        metadata,
        domains.owners(),
        SceneValidationLimits::default(),
        control,
    )
    .expect("the minimal profile fixture must validate")
}

#[test]
fn compiled_profile_facts_do_not_borrow_authored_storage() {
    let mut control = TestControl::default();
    let compiled = {
        let resources = Vec::from(resource_rows());
        let sources = vec![source_row(RESOURCE_ID)];
        let provenance = vec![ProvenanceNodeAuthoring::source(SOURCE_ID)];
        let scene_provenance = vec![SOURCE_PROVENANCE];
        let synthetic_root_provenance = vec![SOURCE_PROVENANCE];
        let node_provenance = vec![SOURCE_PROVENANCE];
        let owners = mandatory_owner_rows(
            &scene_provenance,
            &synthetic_root_provenance,
            &node_provenance,
        );
        let metadata = vec![MetadataAuthoringEntry::new(
            ProfileOwner::Node(NodeLocalId::new(0)),
            &vocabulary().metadata_key,
            ExtensionValueRef::F64(2.0),
        )];

        validate_minimal_profile(
            LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
            MetadataAuthoringBatch::new(&metadata),
            &mut control,
        )
        .into_compiled_facts_for_test()
    };

    assert_eq!(compiled.0, [1, 1, 1, 3, 1]);
    assert_eq!(compiled.1, [1, 3, 1]);

    drop(compiled);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn compiled_profile_storage_drops_before_its_reservation() {
    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let provenance = [ProvenanceNodeAuthoring::source(SOURCE_ID)];
    let scene_provenance = [SOURCE_PROVENANCE];
    let synthetic_root_provenance = [SOURCE_PROVENANCE];
    let node_provenance = [SOURCE_PROVENANCE];
    let owners = mandatory_owner_rows(
        &scene_provenance,
        &synthetic_root_provenance,
        &node_provenance,
    );
    let metadata = [MetadataAuthoringEntry::new(
        ProfileOwner::Node(NodeLocalId::new(0)),
        &vocabulary().metadata_key,
        ExtensionValueRef::F64(2.0),
    )];
    let stage = Rc::new(Cell::new(0));
    let mut control = TestControl {
        reservation_drop_stage: Some(Rc::clone(&stage)),
        ..TestControl::default()
    };
    let mut validated = validate_minimal_profile(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
        MetadataAuthoringBatch::new(&metadata),
        &mut control,
    );

    validated.install_storage_drop_probe(Rc::clone(&stage));
    drop(validated);

    assert_eq!(stage.get(), 2);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn stable_resource_identity_requires_fixture_authority_and_binds_exact_identity() {
    let fixture = StableResourceIdentityFixture::ProfileModelV1;
    let evidence = classify_stable_resource_identity_fixture(fixture);
    let observed = ObservedResourceAuthoring::stable(evidence);

    assert_eq!(
        observed.identity().as_str(),
        "fixtures/profile/stable-model.glb"
    );
    match observed.stability() {
        ResourceIdentityStability::Stable {
            owner,
            schema,
            version,
        } => {
            assert_eq!(owner.as_str(), "baozi-tests:profile-authority");
            assert_eq!(schema.as_str(), "baozi-tests:profile-resource");
            assert_eq!(version.get(), 1);
        }
        ResourceIdentityStability::SessionOpaque(reason) => {
            panic!("fixture authority unexpectedly became session-opaque: {reason:?}")
        }
    }

    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut control);
    let resources = [observed];
    let sources = [source_row(RESOURCE_ID)];
    let provenance = [ProvenanceNodeAuthoring::source(SOURCE_ID)];
    let scene_provenance = [SOURCE_PROVENANCE];
    let synthetic_root_provenance = [SOURCE_PROVENANCE];
    let node_provenance = [SOURCE_PROVENANCE];
    let owners = mandatory_owner_rows(
        &scene_provenance,
        &synthetic_root_provenance,
        &node_provenance,
    );

    let validated = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
        MetadataAuthoringBatch::empty(),
        domains.owners(),
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("authority-backed stable identity must pass profile validation");
    assert!(matches!(
        validated.lineage().resources()[0].stability(),
        ResourceIdentityStability::Stable { .. }
    ));
}

#[test]
fn full_profile_preserves_identity_lineage_owners_and_metadata() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut control);
    let owners = domains.owners();

    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let parents = [SOURCE_PROVENANCE];
    let provenance = [
        ProvenanceNodeAuthoring::source(SOURCE_ID),
        ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
            &vocabulary().operation_kind,
            OptionsDigest::sha256([11; 32]),
            &parents,
            &[],
        )),
    ];
    let scene_provenance = [GENERATED_PROVENANCE];
    let synthetic_root_provenance = [GENERATED_PROVENANCE];
    let node_provenance = [SOURCE_PROVENANCE];
    let owner_rows = mandatory_owner_rows(
        &scene_provenance,
        &synthetic_root_provenance,
        &node_provenance,
    );
    let metadata = [MetadataAuthoringEntry::new(
        ProfileOwner::Node(NodeLocalId::new(0)),
        &vocabulary().metadata_key,
        ExtensionValueRef::F64(2.0),
    )];

    let validated = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows),
        MetadataAuthoringBatch::new(&metadata),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("the complete retained profile must validate");

    assert_eq!(validated.resource_count(), 1);
    assert_eq!(validated.source_count(), 1);
    assert_eq!(validated.operation_count(), 1);
    assert_eq!(validated.provenance_node_count(), 2);
    assert_eq!(validated.parent_edge_count(), 1);
    assert_eq!(validated.owner_attachment_count(), 3);
    assert_eq!(validated.metadata_entry_count(), 1);
    assert_eq!(validated.entity_count(), 4);
    assert_eq!(validated.maximum_depth(), 2);
    assert_eq!(validated.provenance_depth(SOURCE_PROVENANCE), Some(1));
    assert_eq!(validated.provenance_depth(GENERATED_PROVENANCE), Some(2));

    let source = validated
        .source_identity(SOURCE_ID)
        .expect("the source identity must remain inspectable");
    assert_eq!(
        source.resource().identity().as_str(),
        "fixtures/profile/model.glb"
    );
    assert_eq!(
        source.resource().stability(),
        &ResourceIdentityStability::SessionOpaque(SessionOpaqueReason::CallerLabel)
    );
    assert_eq!(source.source().format().as_str(), "org.baozi.gltf");
    assert_eq!(
        source.source().content_digest(),
        Some(ContentDigest::sha256([7; 32]))
    );
    assert_eq!(validated.source_leaf(SOURCE_PROVENANCE), Some(SOURCE_ID));

    let operation = validated
        .operation(GENERATED_PROVENANCE)
        .expect("the generated operation must remain inspectable");
    assert_eq!(operation.kind(), &OperationKind::Generated);
    assert_eq!(operation.options_digest(), OptionsDigest::sha256([11; 32]));
    assert_eq!(operation.parents(), &[SOURCE_PROVENANCE]);
    assert_eq!(
        validated.owner_provenance(CanonicalProfileOwner::Scene),
        Some(scene_provenance.as_slice())
    );
    assert_eq!(
        validated.owner_provenance(CanonicalProfileOwner::SyntheticRoot),
        Some(synthetic_root_provenance.as_slice())
    );
    assert_eq!(
        validated.owner_provenance(CanonicalProfileOwner::Node(1)),
        Some(node_provenance.as_slice())
    );

    let metadata = validated
        .metadata_entry(0)
        .expect("the metadata entry must remain inspectable");
    assert_eq!(metadata.owner(), CanonicalProfileOwner::Node(1));
    assert_eq!(metadata.key().as_str(), "gltf:generator_version");
    assert_eq!(metadata.value(), ExtensionValueRef::F64(2.0));

    drop(validated);
    drop(domains);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn duplicate_and_forward_operation_parents_are_rejected() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut control);
    let owners = domains.owners();
    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let scene = [SOURCE_PROVENANCE];
    let synthetic_root = [SOURCE_PROVENANCE];
    let node = [SOURCE_PROVENANCE];
    let owner_rows = mandatory_owner_rows(&scene, &synthetic_root, &node);

    let duplicate_parents = [SOURCE_PROVENANCE, SOURCE_PROVENANCE];
    let duplicate_provenance = [
        ProvenanceNodeAuthoring::source(SOURCE_ID),
        ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
            &vocabulary().operation_kind,
            OptionsDigest::sha256([0; 32]),
            &duplicate_parents,
            &[],
        )),
    ];
    let duplicate = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &duplicate_provenance, &owner_rows),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(matches!(
        duplicate,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::DuplicateParent { node, parent }
        )) if node == GENERATED_PROVENANCE.index() && parent == SOURCE_PROVENANCE.index()
    ));

    let forward_parents = [GENERATED_PROVENANCE];
    let forward_provenance = [
        ProvenanceNodeAuthoring::source(SOURCE_ID),
        ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
            &vocabulary().operation_kind,
            OptionsDigest::sha256([0; 32]),
            &forward_parents,
            &[],
        )),
    ];
    let forward = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &forward_provenance, &owner_rows),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(matches!(
        forward,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::ParentNotBeforeChild { node, parent }
        )) if node == GENERATED_PROVENANCE.index() && parent == GENERATED_PROVENANCE.index()
    ));

    drop(domains);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn owner_provenance_attachments_must_be_strictly_increasing() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut control);
    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let operation_parents = [SOURCE_PROVENANCE];
    let provenance = [
        ProvenanceNodeAuthoring::source(SOURCE_ID),
        ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
            &vocabulary().operation_kind,
            OptionsDigest::sha256([0; 32]),
            &operation_parents,
            &[],
        )),
    ];
    let out_of_order = [GENERATED_PROVENANCE, SOURCE_PROVENANCE];
    let synthetic_root = [SOURCE_PROVENANCE];
    let node = [SOURCE_PROVENANCE];
    let owners = mandatory_owner_rows(&out_of_order, &synthetic_root, &node);

    let result = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
        MetadataAuthoringBatch::empty(),
        domains.owners(),
        SceneValidationLimits::default(),
        &mut control,
    );

    assert!(matches!(
        result,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::OwnerProvenanceOrder {
                owner: CanonicalProfileOwner::Scene,
                previous,
                current,
            }
        )) if previous == GENERATED_PROVENANCE.index() && current == SOURCE_PROVENANCE.index()
    ));

    drop(domains);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn missing_synthetic_root_owner_is_rejected() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut control);
    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let provenance = [ProvenanceNodeAuthoring::source(SOURCE_ID)];
    let attachment = [SOURCE_PROVENANCE];
    let owner_rows = [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &attachment),
        OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(0)), &attachment),
    ];

    let result = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows),
        MetadataAuthoringBatch::empty(),
        domains.owners(),
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(matches!(
        result,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::MissingOwnerProvenance {
                owner: CanonicalProfileOwner::SyntheticRoot
            }
        ))
    ));

    drop(domains);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn foreign_resource_source_and_provenance_references_are_rejected() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut control);
    let owners = domains.owners();
    let resources = resource_rows();
    let attachment = [SOURCE_PROVENANCE];
    let owner_rows = mandatory_owner_rows(&attachment, &attachment, &attachment);

    let foreign_resource_sources = [source_row(ResourceIdentityLocalId::new(1))];
    let source_provenance = [ProvenanceNodeAuthoring::source(SOURCE_ID)];
    let foreign_resource = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(
            &resources,
            &foreign_resource_sources,
            &source_provenance,
            &owner_rows,
        ),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(matches!(
        foreign_resource,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::ResourceOutOfRange {
                source_index: 0,
                resource: 1,
                resource_count: 1
            }
        ))
    ));

    let sources = [source_row(RESOURCE_ID)];
    let foreign_source_provenance = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
        1,
    ))];
    let foreign_source = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(
            &resources,
            &sources,
            &foreign_source_provenance,
            &owner_rows,
        ),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(matches!(
        foreign_source,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::SourceIdentityOutOfRange {
                node: 0,
                source_index: 1,
                source_count: 1
            }
        ))
    ));

    let foreign_attachment = [ProvenanceLocalId::new(1)];
    let foreign_owner_rows = mandatory_owner_rows(&foreign_attachment, &attachment, &attachment);
    let foreign_provenance = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(
            &resources,
            &sources,
            &source_provenance,
            &foreign_owner_rows,
        ),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(matches!(
        foreign_provenance,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::ProvenanceOutOfRange {
                provenance: 1,
                provenance_count: 1,
                ..
            }
        ))
    ));

    drop(domains);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn duplicate_metadata_keys_and_non_finite_values_are_rejected() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut control);
    let owners = domains.owners();
    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let provenance = [ProvenanceNodeAuthoring::source(SOURCE_ID)];
    let attachment = [SOURCE_PROVENANCE];
    let owner_rows = mandatory_owner_rows(&attachment, &attachment, &attachment);
    let duplicate_metadata = [
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &vocabulary().metadata_key,
            ExtensionValueRef::U64(1),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &vocabulary().metadata_key,
            ExtensionValueRef::U64(2),
        ),
    ];

    let duplicate = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows),
        MetadataAuthoringBatch::new(&duplicate_metadata),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(matches!(
        duplicate,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::DuplicateMetadataKey { .. }
        ))
    ));

    let non_finite_metadata = [MetadataAuthoringEntry::new(
        ProfileOwner::Scene,
        &vocabulary().metadata_key,
        ExtensionValueRef::F64(f64::NAN),
    )];
    let non_finite = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows),
        MetadataAuthoringBatch::new(&non_finite_metadata),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(matches!(
        non_finite,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::MetadataNonFinite { entry: 0 }
        ))
    ));

    let baseline = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("the profile without metadata must validate");
    let baseline_cells = baseline.payload_cell_count();
    drop(baseline);
    let vector_key = NamespacedName::new("gltf:vectors").expect("metadata key must be canonical");
    let vectors = [Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)];
    let dense_metadata = [MetadataAuthoringEntry::new(
        ProfileOwner::Scene,
        &vector_key,
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Vec3(&vectors)),
    )];
    let dense = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows),
        MetadataAuthoringBatch::new(&dense_metadata),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("finite dense vector metadata must validate");
    assert_eq!(
        dense.payload_cell_count() - baseline_cells,
        u64::try_from(vector_key.as_str().len()).expect("key length must fit") + 6,
        "dense vector cells must count every scalar component"
    );
    drop(dense);

    drop(domains);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn metadata_owner_buckets_are_linear_and_owner_local_keys_must_be_ordered() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut control);
    let owners = domains.owners();
    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let provenance = [ProvenanceNodeAuthoring::source(SOURCE_ID)];
    let attachment = [SOURCE_PROVENANCE];
    let owner_rows = mandatory_owner_rows(&attachment, &attachment, &attachment);
    let first_key = NamespacedName::new("gltf:first").expect("metadata key must be canonical");
    let second_key = NamespacedName::new("gltf:second").expect("metadata key must be canonical");
    let interleaved = [
        MetadataAuthoringEntry::new(ProfileOwner::Scene, &first_key, ExtensionValueRef::U64(1)),
        MetadataAuthoringEntry::new(
            ProfileOwner::Node(NodeLocalId::new(0)),
            &first_key,
            ExtensionValueRef::U64(2),
        ),
        MetadataAuthoringEntry::new(ProfileOwner::Scene, &second_key, ExtensionValueRef::U64(3)),
        MetadataAuthoringEntry::new(
            ProfileOwner::Node(NodeLocalId::new(0)),
            &second_key,
            ExtensionValueRef::U64(4),
        ),
    ];
    let lineage = LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows);
    let validated = validate_profile_batch_with_control(
        lineage,
        MetadataAuthoringBatch::new(&interleaved),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("interleaved owners with ordered local keys must validate");
    let canonical = (0..4)
        .map(|index| {
            let entry = validated
                .metadata_entry(index)
                .expect("canonical metadata row must exist");
            (entry.owner(), entry.key().as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        canonical,
        [
            (CanonicalProfileOwner::Scene, "gltf:first"),
            (CanonicalProfileOwner::Scene, "gltf:second"),
            (CanonicalProfileOwner::Node(1), "gltf:first"),
            (CanonicalProfileOwner::Node(1), "gltf:second"),
        ]
    );
    drop(validated);

    let reversed = [
        MetadataAuthoringEntry::new(ProfileOwner::Scene, &second_key, ExtensionValueRef::U64(1)),
        MetadataAuthoringEntry::new(ProfileOwner::Scene, &first_key, ExtensionValueRef::U64(2)),
    ];
    let rejected = validate_profile_batch_with_control(
        lineage,
        MetadataAuthoringBatch::new(&reversed),
        owners,
        SceneValidationLimits::default(),
        &mut control,
    );
    assert!(matches!(
        rejected,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::MetadataKeyOrder {
                previous: 0,
                entry: 1,
                owner: CanonicalProfileOwner::Scene,
            }
        ))
    ));

    drop(domains);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn exact_canonical_limit_succeeds_and_one_over_is_rejected() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut control);
    let owners = domains.owners();
    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let provenance = [ProvenanceNodeAuthoring::source(SOURCE_ID)];
    let attachment = [SOURCE_PROVENANCE];
    let owner_rows = mandatory_owner_rows(&attachment, &attachment, &attachment);
    let metadata = [MetadataAuthoringEntry::new(
        ProfileOwner::Scene,
        &vocabulary().metadata_key,
        ExtensionValueRef::String("2.0"),
    )];
    let lineage = LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows);
    let metadata = MetadataAuthoringBatch::new(&metadata);
    let nonce_count_before = issued_profile_segment_nonce_count_for_test();

    let baseline = validate_profile_batch_with_control(
        lineage,
        metadata,
        owners,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("the baseline profile must validate");
    let exact_bytes = baseline.canonical_bytes();
    drop(baseline);

    let exact = validate_profile_batch_with_control(
        lineage,
        metadata,
        owners,
        SceneValidationLimits::default().with_max_canonical_bytes(exact_bytes),
        &mut control,
    )
    .expect("the exact canonical-byte limit must be inclusive");
    assert_eq!(exact.canonical_bytes(), exact_bytes);
    drop(exact);

    let one_over = validate_profile_batch_with_control(
        lineage,
        metadata,
        owners,
        SceneValidationLimits::default().with_max_canonical_bytes(exact_bytes - 1),
        &mut control,
    );
    assert!(matches!(
        one_over,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::LimitExceeded {
                limit: ProfileAuthoringLimit::CanonicalBytes,
                actual,
                maximum,
            }
        )) if actual == exact_bytes && maximum == exact_bytes - 1
    ));
    assert_eq!(
        issued_profile_segment_nonce_count_for_test(),
        nonce_count_before,
        "authoring preflight and exact-budget rejection must not issue a segment nonce",
    );

    drop(domains);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn profile_work_budget_is_preflighted_and_exact() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut domain_control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut domain_control);
    let owners = domains.owners();
    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let provenance = [ProvenanceNodeAuthoring::source(SOURCE_ID)];
    let attachment = [SOURCE_PROVENANCE];
    let owner_rows = mandatory_owner_rows(&attachment, &attachment, &attachment);
    let lineage = LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows);

    let mut baseline_control = TestControl::default();
    let baseline = validate_profile_batch_with_control(
        lineage,
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut baseline_control,
    )
    .expect("the baseline profile must validate");
    let exact_work = baseline.validation_work();
    drop(baseline);

    let mut exact_control = TestControl::default();
    let exact = validate_profile_batch_with_control(
        lineage,
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default().with_max_work(exact_work),
        &mut exact_control,
    )
    .expect("the exact profile work limit must be inclusive");
    assert_eq!(exact.validation_work(), exact_work);
    drop(exact);

    let mut one_short_control = TestControl::default();
    let one_short = validate_profile_batch_with_control(
        lineage,
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default().with_max_work(exact_work - 1),
        &mut one_short_control,
    );
    assert!(matches!(
        one_short,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::LimitExceeded {
                limit: ProfileAuthoringLimit::Work,
                actual,
                maximum,
            }
        )) if actual == exact_work && maximum == exact_work - 1
    ));

    let mut zero_control = TestControl::default();
    let zero = validate_profile_batch_with_control(
        lineage,
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default().with_max_work(0),
        &mut zero_control,
    );
    assert!(matches!(
        zero,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::LimitExceeded {
                limit: ProfileAuthoringLimit::Work,
                maximum: 0,
                ..
            }
        ))
    ));
    assert_eq!(zero_control.reserves, 0);
    assert_eq!(zero_control.work, 0);

    drop(domains);
    assert_eq!(domain_control.live.get(), 0);
    assert_eq!(baseline_control.live.get(), 0);
    assert_eq!(exact_control.live.get(), 0);
    assert_eq!(one_short_control.live.get(), 0);
    assert_eq!(zero_control.live.get(), 0);
}

#[test]
fn owner_plan_initialization_is_cancellable_midstream() {
    const OWNER_COUNT: u32 = 300;

    let roots = (0..OWNER_COUNT).map(NodeLocalId::new).collect::<Vec<_>>();
    let nodes = (0..OWNER_COUNT)
        .map(|_| trs(Presence::Absent))
        .collect::<Vec<_>>();
    let mut domain_control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut domain_control);
    let owners = domains.owners();
    let mut profile_control = TestControl {
        cancel_on_poll: Some(2),
        ..TestControl::default()
    };
    let cancelled = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&[], &[], &[], &[]),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut profile_control,
    );
    assert!(matches!(
        cancelled,
        Err(ProfileAuthoringFailure::Control(
            TestControlError::Cancelled
        ))
    ));
    assert_eq!(profile_control.polls, 2);
    assert_eq!(profile_control.work, 256);
    assert_eq!(profile_control.live.get(), 0);

    drop(domains);
    assert_eq!(domain_control.live.get(), 0);
}

#[test]
fn missing_owner_scan_meters_and_polls_a_long_matched_prefix() {
    const NODE_COUNT: u32 = 300;

    let roots = (0..NODE_COUNT).map(NodeLocalId::new).collect::<Vec<_>>();
    let nodes = (0..NODE_COUNT)
        .map(|_| trs(Presence::Absent))
        .collect::<Vec<_>>();
    let mut domain_control = TestControl::default();
    let domains = validated_domains(&roots, &nodes, &mut domain_control);
    let owners = domains.owners();
    let last_owner = owners
        .owner_at(owners.owner_count() - 1)
        .expect("the nonempty owner catalog must have a final row");
    let resources = resource_rows();
    let sources = [source_row(RESOURCE_ID)];
    let provenance = [ProvenanceNodeAuthoring::source(SOURCE_ID)];
    let attachment = [SOURCE_PROVENANCE];
    let owner_rows = std::iter::once(ProfileOwner::Scene)
        .chain(std::iter::once(ProfileOwner::SyntheticRoot))
        .chain((0..NODE_COUNT).map(|index| ProfileOwner::Node(NodeLocalId::new(index))))
        .map(|owner| OwnerProvenanceAuthoring::new(owner, &attachment))
        .collect::<Vec<_>>();

    let mut first_missing_control = TestControl::default();
    let first_missing = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &owner_rows[1..]),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut first_missing_control,
    );
    assert!(matches!(
        first_missing,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::MissingOwnerProvenance {
                owner: CanonicalProfileOwner::Scene,
            }
        ))
    ));

    let mut last_missing_control = TestControl::default();
    let last_missing = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(
            &resources,
            &sources,
            &provenance,
            &owner_rows[..owner_rows.len() - 1],
        ),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut last_missing_control,
    );
    assert!(matches!(
        last_missing,
        Err(ProfileAuthoringFailure::Semantic(
            ProfileAuthoringSemanticError::MissingOwnerProvenance {
                owner,
            }
        )) if owner == last_owner
    ));
    assert_eq!(
        last_missing_control.work - first_missing_control.work,
        u64::from(NODE_COUNT) + 1,
        "every matched owner before the missing tail row must consume traversal work",
    );
    assert!(last_missing_control.polls > first_missing_control.polls);

    let mut cancelled_control = TestControl {
        cancel_on_poll: Some(last_missing_control.polls),
        ..TestControl::default()
    };
    let cancelled = validate_profile_batch_with_control(
        LineageAuthoringBatch::new(
            &resources,
            &sources,
            &provenance,
            &owner_rows[..owner_rows.len() - 1],
        ),
        MetadataAuthoringBatch::empty(),
        owners,
        SceneValidationLimits::default(),
        &mut cancelled_control,
    );
    assert!(matches!(
        cancelled,
        Err(ProfileAuthoringFailure::Control(
            TestControlError::Cancelled
        ))
    ));
    assert!(cancelled_control.work >= first_missing_control.work);
    assert!(cancelled_control.work <= last_missing_control.work);
    assert_eq!(cancelled_control.polls, last_missing_control.polls);

    drop(domains);
    assert_eq!(domain_control.live.get(), 0);
    assert_eq!(first_missing_control.live.get(), 0);
    assert_eq!(last_missing_control.live.get(), 0);
    assert_eq!(cancelled_control.live.get(), 0);
}
