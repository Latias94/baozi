use super::*;
use crate::canonical_scene::store::construction::node_scene::semantic::profile::NodeSceneProfileTables;
use crate::canonical_scene::store::construction::node_scene::tests::profile_fixture;
use crate::canonical_scene::store::typed_storage::edit::space::SceneSpacePlan;
use crate::canonical_scene::store::typed_storage::edit::{
    GeometryTriangulationPlan, TriangulationOrigins,
};
use crate::space_conversion::{SpaceConversion, TargetSpace};

const JSON: &[u8] = br#" { "array": [1e+02, null, {"escaped":"\u0061"}] } "#;

fn source_scene(account: &PhysicalStorageAccount, polygon: bool) -> NodeSceneSuccessParts {
    let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    let normals = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    let tangents = [
        1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0,
    ];
    let attributes = [
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
            &normals,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Tangent,
            AttributeDomain::Vertex,
            4,
            &tangents,
            Presence::Absent,
        ),
    ];
    let indices: &[u32] = if polygon {
        &[0, 1, 2, 3]
    } else {
        &[0, 1, 2, 0, 2, 3]
    };
    let topology = if polygon {
        GeometryTopologyAuthoring::Polygons(&[4])
    } else {
        GeometryTopologyAuthoring::TriangleList
    };
    let primitives = [GeometryPrimitiveAuthoring::new(
        topology,
        Presence::Present(indices),
        Presence::Absent,
        AuthoringRange::new(0, 3),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let deformation =
        DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &primitive_deformations, &[]);
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::STANDARD_TARGET, &[], &[]);
    let nodes = single_trs_batch();
    let appearance = AppearanceAuthoringBatch::empty();
    let animation = AnimationAuthoringBatch::empty();
    let fixture = profile_fixture(spatial, nodes, appearance, geometry, deformation, animation);
    let format = FormatId::new("org.baozi.source-metadata").unwrap();
    let locators: [_; 3] = std::array::from_fn(|index| {
        SourceObjectLocator::indexed(SourceObjectKind::Document, index as u64)
    });
    let sources = locators.each_ref().map(|locator| {
        SourceIdentityAuthoring::new(ResourceIdentityLocalId::new(0), &format, locator)
    });
    let key = NamespacedName::new("test:extras").unwrap();
    let mime = ExtensionMediaType::new("application/json").unwrap();
    let value = ExtensionValueRef::Bytes {
        media_type: &mime,
        payload: JSON,
    };
    let metadata = [
        MetadataAuthoringEntry::new(ProfileOwner::Attribute(2), &key, value),
        MetadataAuthoringEntry::for_source(SourceIdentityLocalId::new(2), &key, value),
    ];
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(
            fixture.batch().lineage().resources(),
            &sources,
            fixture.batch().lineage().provenance_nodes(),
            fixture.batch().lineage().owner_provenance(),
        ),
        MetadataAuthoringBatch::new(&metadata),
    );
    successful_parts(
        NodeSceneTestTransaction::with_account(
            account,
            test_limits(),
            SceneAuthoringBatch::new(
                spatial,
                nodes,
                appearance,
                geometry,
                deformation,
                animation,
                profile,
            ),
            SceneValidationLimits::default(),
        )
        .unwrap()
        .finish_draft()
        .unwrap()
        .publish_for_test()
        .unwrap(),
    )
}

fn assert_source_payload(profile: &ProfileStorage, expected_entries: usize) {
    let entries = &profile.state.as_ref().metadata_entries;
    assert_eq!(entries.len(), expected_entries);
    let entry = entries
        .iter()
        .find(|entry| entry.owner == CanonicalMetadataOwner::SourceObject(2))
        .unwrap();
    assert_eq!(
        entry.key.get_str(&profile.metadata.as_ref().text),
        Some("test:extras")
    );
    let StoredMetadataValue::Bytes {
        media_type,
        payload,
    } = entry.value
    else {
        panic!()
    };
    assert_eq!(
        media_type.get_str(&profile.metadata.as_ref().text),
        Some("application/json")
    );
    assert_eq!(payload.get(&profile.metadata.as_ref().bytes), Some(JSON));
    let tables = NodeSceneProfileTables::try_new(profile).unwrap();
    // The final source remains addressable before the appended empty-source edit segment.
    assert_eq!(tables.source_at(2).unwrap().index(), 2);
    assert_eq!(
        tables.source_at(2).unwrap().format(),
        "org.baozi.source-metadata"
    );
    assert_eq!(
        tables.resource_at(0).unwrap().identity(),
        "fixtures/node-scene/input"
    );
    assert!(tables.source_at(3).is_err());
    assert!(tables.resource_at(1).is_err());
}

#[test]
fn normal_deletion_keeps_same_index_source_metadata_and_exact_json_after_source_drop() {
    let factory = test_factory();
    let account = test_account(&factory, 1024);
    let source = source_scene(&account, false);
    let geometry = GeometryNormalsPlan::try_new(
        source.scene.read_geometry(),
        0,
        &GENERATED_NORMALS,
        NormalWritePolicy::Replace,
        TangentPolicy::Remove,
        &mut Control::default(),
    )
    .unwrap();
    assert_eq!(geometry.attribute_mapping().source_tangent(), Some(2));
    let pending = BoundPendingNodeSceneEdit::try_normals(
        &source.scene,
        geometry,
        false,
        OptionsDigest::sha256([61; 32]),
        &mut || Ok(()),
    )
    .unwrap();
    let plan = pending.counts.profile;
    let output = retain_edit(pending);
    assert_valid_profile(&output.output().profile, plan);
    assert_source_payload(&output.output().profile, 1);
    assert!(
        !output
            .output()
            .profile
            .state
            .as_ref()
            .metadata_entries
            .iter()
            .any(|entry| entry.owner
                == CanonicalMetadataOwner::Entity(CanonicalProfileOwner::Attribute(2)))
    );
    drop(source);
    assert_source_payload(&output.output().profile, 1);
    drop(output);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn triangulation_shares_source_json_and_keeps_direct_lookup_past_empty_tail_segment() {
    let factory = test_factory();
    let account = test_account(&factory, 1024);
    let source = source_scene(&account, true);
    let triangles = [[0, 1, 2], [0, 2, 3]];
    let geometry = GeometryTriangulationPlan::try_new(
        source.scene.read_geometry(),
        0,
        TriangulationOrigins {
            triangles: &triangles,
            face_offsets: &[0, 2],
        },
        &mut Control::default(),
    )
    .unwrap();
    let pending = BoundPendingNodeSceneEdit::try_triangulation(
        &source.scene,
        geometry,
        OptionsDigest::sha256([62; 32]),
        &mut || Ok(()),
    )
    .unwrap();
    let plan = pending.counts.profile;
    let output = retain_edit(pending);
    assert_valid_profile(&output.output().profile, plan);
    assert!(std::ptr::eq(
        source.scene.tables.profile.metadata.as_ref(),
        output.output().profile.metadata.as_ref()
    ));
    drop(source);
    assert_source_payload(&output.output().profile, 2);
    drop(output);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn space_conversion_shares_exact_source_json_and_keeps_source_rows_independent_of_entities() {
    let factory = test_factory();
    let account = test_account(&factory, 1024);
    let source = source_scene(&account, false);
    let target = TargetSpace::try_new(
        crate::Handedness::Right,
        crate::Axis::PositiveY,
        crate::Axis::NegativeZ,
        0.5,
        crate::FrontFaceWinding::CounterClockwise,
    )
    .unwrap();
    let conversion = SpaceConversion::try_new(&SceneSpace::STANDARD_TARGET, target, None).unwrap();
    let space =
        SceneSpacePlan::try_new(&source.scene.tables, conversion, &mut Control::default()).unwrap();
    let pending = BoundPendingNodeSceneEdit::try_space_conversion(
        &source.scene,
        space,
        OptionsDigest::sha256([63; 32]),
        &mut || Ok(()),
    )
    .unwrap();
    let plan = pending.counts.profile;
    let output = retain_edit(pending);
    assert_valid_profile(&output.output().profile, plan);
    assert!(std::ptr::eq(
        source.scene.tables.profile.metadata.as_ref(),
        output.output().profile.metadata.as_ref()
    ));
    drop(source);
    assert_source_payload(&output.output().profile, 2);
    drop(output);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}
