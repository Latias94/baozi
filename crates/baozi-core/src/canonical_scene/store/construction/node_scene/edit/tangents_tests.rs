//! Tangent owner remapping and exact dependency ancestry against a retained source Scene.

use super::*;
use crate::attribute::{
    AttributeSemantic, TangentBasisConvention, TangentConventions, TangentHandednessEncoding,
};
use crate::canonical_scene::store::construction::node_scene::edit::plan::BoundPendingNodeSceneEdit;
use crate::canonical_scene::store::construction::node_scene::tests::{
    single_trs_batch, successful_parts, test_account, test_factory, test_limits,
};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneSuccessParts, NodeSceneTestTransaction,
};
use crate::canonical_scene::store::production::*;
use crate::canonical_scene::store::typed_storage::edit::VectorValues;
use crate::canonical_scene::store::typed_storage::edit::tangents::GeometryTangentsPlan;
use crate::extension::ExtensionValueRef;
use crate::identity::SceneGenerationId;
use crate::namespaced::NamespacedName;
use crate::provenance::{OptionsDigest, ResourceIdentity, SourceObjectKind, SourceObjectLocator};
use crate::scene_space::SceneSpace;
use crate::{FormatId, SceneValidationLimits};
use baozi_operation::PhysicalStorageAccount;
use std::convert::Infallible;
use std::sync::atomic::AtomicU64;

const TANGENTS: [[f32; 4]; 6] = [[1.0, 0.0, 0.0, 1.0]; 6];
const CONVENTIONS: TangentConventions = TangentConventions {
    basis: TangentBasisConvention::TriangleUvDerivative,
    encoding: TangentHandednessEncoding::SignInW,
};

#[derive(Default)]
struct Control(u64);

impl TypedStorageBuildControl for Control {
    type Error = Infallible;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.0 += 1;
        Ok(action())
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.run_unit(action)
    }
}

fn source_scene(account: &PhysicalStorageAccount, had_tangent: bool) -> NodeSceneSuccessParts {
    let positions = [0.0; 12];
    let second_positions = [0.0; 9];
    let vertex_normals = [0.0, 0.0, 1.0].repeat(4);
    let corner_normals = [0.0, 0.0, 1.0].repeat(6);
    let old_tangents = [1.0, 0.0, 0.0, -1.0].repeat(4);
    let vertex_uvs = [0.0; 8];
    let corner_uvs = [0.0; 12];
    let mut attributes = vec![
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            &vertex_normals,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            3,
            &corner_normals,
            Presence::Absent,
        ),
    ];
    if had_tangent {
        attributes.push(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Tangent,
            AttributeDomain::Vertex,
            4,
            &old_tangents,
            Presence::Absent,
        ));
    }
    attributes.push(GeometryAttributeAuthoring::f32(
        AttributeSemantic::Texcoord(7),
        AttributeDomain::Vertex,
        2,
        &vertex_uvs,
        Presence::Absent,
    ));
    attributes.push(GeometryAttributeAuthoring::f32(
        AttributeSemantic::Texcoord(7),
        AttributeDomain::Corner,
        2,
        &corner_uvs,
        Presence::Absent,
    ));
    let second = attributes.len() as u32;
    attributes.push(GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &second_positions,
        Presence::Absent,
    ));
    let indices = [0, 1, 2, 0, 2, 3];
    let primitives = [
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&indices),
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, second),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Absent,
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(second, 1),
        ),
    ];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 2))];
    let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[]); 2];
    let deformation =
        DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &primitive_deformations, &[]);
    let mut owners = vec![
        ProfileOwner::Scene,
        ProfileOwner::SyntheticRoot,
        ProfileOwner::Node(NodeLocalId::new(0)),
        ProfileOwner::LogicalMesh(0),
        ProfileOwner::Primitive(0),
        ProfileOwner::Primitive(1),
    ];
    owners.extend((0..attributes.len() as u32).map(ProfileOwner::Attribute));
    let references: Vec<_> = (0..owners.len() as u32)
        .map(|index| [ProvenanceLocalId::new(index)])
        .collect();
    let attachments: Vec<_> = owners
        .iter()
        .copied()
        .zip(&references)
        .map(|(owner, refs)| OwnerProvenanceAuthoring::new(owner, refs))
        .collect();
    let provenance: Vec<_> = owners
        .iter()
        .map(|_| ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(0)))
        .collect();
    let resource = ResourceIdentity::new("fixtures/tangents/dependencies").unwrap();
    let format = FormatId::new("org.baozi.fixture").unwrap();
    let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
    let resources = [ObservedResourceAuthoring::session_opaque(
        &resource,
        SessionOpaqueReason::CallerLabel,
    )];
    let sources = [SourceIdentityAuthoring::new(
        ResourceIdentityLocalId::new(0),
        &format,
        &locator,
    )];
    let key = NamespacedName::new("test:retained").unwrap();
    let mut metadata = vec![MetadataAuthoringEntry::new(
        ProfileOwner::Attribute(second),
        &key,
        ExtensionValueRef::String("other primitive"),
    )];
    if had_tangent {
        metadata.push(MetadataAuthoringEntry::new(
            ProfileOwner::Attribute(3),
            &key,
            ExtensionValueRef::String("old tangent"),
        ));
    }
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &attachments),
        MetadataAuthoringBatch::new(&metadata),
    );
    let input = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        single_trs_batch(),
        AppearanceAuthoringBatch::empty(),
        geometry,
        deformation,
        AnimationAuthoringBatch::empty(),
        profile,
    );
    let mut invocation_root_1 =
        crate::canonical_scene::store::construction::SceneCanonicalInvocationHost::new();
    let draft = NodeSceneTestTransaction::with_account(
        &mut invocation_root_1,
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .unwrap()
    .finish_draft()
    .unwrap();
    successful_parts(
        draft
            .publish_with_generation_source_for_test(|| {
                SceneGenerationId::reserve_from(&AtomicU64::new(1))
            })
            .unwrap(),
    )
}

#[test]
fn retained_tangent_ancestry_uses_only_the_selected_domains_and_maps_all_owners() {
    for had_tangent in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        let source = source_scene(&account, had_tangent);
        let geometry = GeometryTangentsPlan::try_new(
            source.scene.read_geometry(),
            0,
            VectorValues::F32(&TANGENTS),
            had_tangent,
            AttributeDomain::Vertex,
            7,
            AttributeDomain::Corner,
            &CONVENTIONS,
            &mut Control::default(),
        )
        .unwrap();
        let mapping = geometry.attribute_mapping();
        let pending = BoundPendingNodeSceneEdit::try_tangents(
            &source.scene,
            geometry,
            OptionsDigest::sha256([73; 32]),
            &mut || Ok(()),
        )
        .unwrap();
        let plan = pending.counts.profile;
        assert_eq!(plan.target_count.as_u32(), 2);
        assert_eq!(
            plan.output_counts.owner_count,
            source.scene.proof.profile_counts().owner_count + u32::from(!had_tangent)
        );
        assert_eq!(
            plan.output_counts.metadata_entry_count,
            source.scene.proof.profile_counts().metadata_entry_count
        );
        assert!(!plan.metadata.rewrites_payloads());
        let targets = ProfileEditTargets::Tangents {
            primitive: 0,
            attributes: mapping,
        };
        let mut dependency_owners = vec![
            CanonicalProfileOwner::Primitive(0),
            CanonicalProfileOwner::Attribute(0),
            CanonicalProfileOwner::Attribute(1),
            CanonicalProfileOwner::Attribute(4 + u32::from(had_tangent)),
        ];
        if had_tangent {
            dependency_owners.push(CanonicalProfileOwner::Attribute(3));
        }
        let mut expected = Vec::new();
        let state = source.scene.tables.profile.state.as_ref();
        for owner in dependency_owners {
            let row = profile_owner(state, plan.source_layout, owner).unwrap();
            expected.extend_from_slice(row.attachments.get(&state.attachments).unwrap());
        }
        expected.sort_unstable_by_key(|reference| (reference.segment(), reference.local()));
        expected.dedup();
        let parents = owner_parents(
            &source.scene,
            plan.source_layout,
            targets,
            CanonicalProfileOwner::Attribute(3),
        )
        .unwrap();
        assert_eq!(parents.iter().collect::<Vec<_>>(), expected);
        assert_eq!(expected.len(), 4 + usize::from(had_tangent));
        let mut control = Control::default();
        let lineage = plan
            .build_lineage(
                &source.scene,
                &pending.operation,
                issue_profile_segment_nonce().unwrap(),
                &mut control,
            )
            .unwrap();
        assert_eq!(control.0, plan.lineage_work().unwrap());
        assert_eq!(
            lineage.provenance[1].parents.get(&lineage.parents),
            Some(expected.as_slice())
        );
        assert_eq!(lineage.operations.len(), 2);
        for ordinal in 0..plan.source_layout.owner_count() {
            let source_owner = plan.source_layout.owner_at(ordinal).unwrap();
            let output_owner = targets.output_owner(source_owner).unwrap();
            assert_eq!(targets.source_owner(output_owner), Some(source_owner));
        }
        assert_eq!(
            targets.source_owner(CanonicalProfileOwner::Attribute(3)),
            had_tangent.then_some(CanonicalProfileOwner::Attribute(3))
        );
        assert_eq!(targets.deleted_owner(), None);
        drop(source);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
}
