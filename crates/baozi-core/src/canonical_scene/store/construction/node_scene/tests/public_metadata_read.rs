use baozi_operation::PhysicalStorageAccount;

use super::{profile_fixture, successful_parts, test_account, test_factory, test_limits};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneSuccessParts, NodeSceneTestTransaction,
};
use crate::canonical_scene::store::production::*;
use crate::extension::{ExtensionArrayRef, ExtensionMediaType, ExtensionValueRef};
use crate::provenance::{
    ContentDigest, ResourceIdentity, SourceDisplayName, SourceLocatorValue, SourceObjectLocator,
};
use crate::{
    AuthoredTransform, Color, DigestAlgorithm, FormatId, Mat4, MetadataArrayRef, MetadataOwnerRef,
    MetadataValueRef, NamespacedName, ProfileResourceStabilityRef, ProfileSessionOpaqueReason,
    SceneProfileOwner, SceneSnapshotLimits, SceneSpace, SceneValidationLimits, SourceObjectKind,
    SourceObjectLocatorRef, ValidatedScene, Vec2, Vec3, Vec4,
};

const JSON: &[u8] = br#" { "nested": [null, {"x": 1e+02}], "escape": "\u0061" } "#;
const PRECISE: f64 = f64::from_bits(0x3ff0_0000_0000_0001);

fn build_metadata_scene(account: &PhysicalStorageAccount) -> NodeSceneSuccessParts {
    // The metadata's authored node zero becomes canonical node two.
    let roots = [NodeLocalId::new(1)];
    let node_rows = [
        NodeAuthoringNode::new(
            Presence::Present(NodeParent::new(NodeLocalId::new(1), 0)),
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let nodes = NodeAuthoringBatch::new(Presence::Present(&roots), &node_rows);
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::empty();
    let deformation = DeformationAuthoringBatch::empty();
    let animation = AnimationAuthoringBatch::empty();
    let fixture = profile_fixture(spatial, nodes, appearance, geometry, deformation, animation);
    let identities = [
        ResourceIdentity::new("models/scene.glb").unwrap(),
        ResourceIdentity::new("models/buffer.bin").unwrap(),
    ];
    let resources = identities.each_ref().map(|identity| {
        ObservedResourceAuthoring::session_opaque(identity, SessionOpaqueReason::CallerLabel)
    });
    let format = FormatId::new("org.baozi.gltf").unwrap();
    let locators = [
        SourceObjectLocator::indexed(SourceObjectKind::Document, u64::MAX),
        SourceObjectLocator::try_namespaced(
            Some(crate::ExtensionSchemaId::new("test:gltf-object", 2).unwrap()),
            SourceLocatorValue::new("accessors/7").unwrap(),
        )
        .unwrap(),
    ];
    let display_name = SourceDisplayName::new("Document").unwrap();
    let sources = [
        SourceIdentityAuthoring::new(ResourceIdentityLocalId::new(0), &format, &locators[0])
            .with_display_name(&display_name)
            .with_content_digest(ContentDigest::sha256([7; 32])),
        SourceIdentityAuthoring::new(ResourceIdentityLocalId::new(1), &format, &locators[1]),
    ];
    let media_type = ExtensionMediaType::new("application/json").unwrap();
    let keys: [_; 20] =
        std::array::from_fn(|index| NamespacedName::new(format!("test:v{index:02}")).unwrap());
    let bools = [false, true];
    let signed = [i64::MIN, i64::MAX];
    let unsigned = [0, u64::MAX];
    let doubles = [-0.0, PRECISE];
    let vec2s = [Vec2::new(-0.0, f32::from_bits(1))];
    let vec3s = [Vec3::new(-0.0, 1.0, 2.0)];
    let vec4s = [Vec4::new(0.0, -0.0, 2.0, 3.0)];
    let colors = [Color::linear_rgba(0.0, -0.0, 0.5, 1.0)];
    let matrices = [Mat4::IDENTITY];
    let values = [
        ExtensionValueRef::Bool(true),
        ExtensionValueRef::I64(i64::MIN),
        ExtensionValueRef::U64(u64::MAX),
        ExtensionValueRef::F64(-0.0),
        ExtensionValueRef::String("text α"),
        ExtensionValueRef::Bytes {
            media_type: &media_type,
            payload: JSON,
        },
        ExtensionValueRef::Vec2(vec2s[0]),
        ExtensionValueRef::Vec3(vec3s[0]),
        ExtensionValueRef::Vec4(vec4s[0]),
        ExtensionValueRef::Color(colors[0]),
        ExtensionValueRef::Mat4(matrices[0]),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Bool(&bools)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::I64(&signed)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::U64(&unsigned)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::F64(&doubles)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Vec2(&vec2s)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Vec3(&vec3s)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Vec4(&vec4s)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Color(&colors)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Mat4(&matrices)),
    ];
    let json_key = NamespacedName::new("test:json").unwrap();
    let mut entries: Vec<_> = keys
        .iter()
        .zip(values)
        .map(|(key, value)| MetadataAuthoringEntry::new(ProfileOwner::Scene, key, value))
        .collect();
    entries.extend([
        MetadataAuthoringEntry::for_source(
            SourceIdentityLocalId::new(1),
            &json_key,
            ExtensionValueRef::Bytes {
                media_type: &media_type,
                payload: b"null",
            },
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Node(NodeLocalId::new(0)),
            &json_key,
            ExtensionValueRef::String("node annotation"),
        ),
        MetadataAuthoringEntry::for_source(
            SourceIdentityLocalId::new(0),
            &json_key,
            ExtensionValueRef::Bytes {
                media_type: &media_type,
                payload: JSON,
            },
        ),
    ]);
    let provenance = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
        0,
    ))];
    // Source metadata does not require a provenance owner row for either source object.
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(&resources, &sources, &provenance, &fixture.owners),
        MetadataAuthoringBatch::new(&entries),
    );
    let mut construction_root_1 =
        crate::canonical_scene::store::construction::SceneCanonicalInvocationHost::new();
    let parts = successful_parts(
        NodeSceneTestTransaction::with_account(
            &mut construction_root_1,
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
    );
    drop(construction_root_1);
    parts
}

#[test]
fn metadata_readers_borrow_all_value_variants_and_source_fields_without_allocations() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let parts = build_metadata_scene(&account);
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let allocations = allocation_counter::measure(|| {
        assert_eq!(scene.metadata_count(), 23);
        assert_eq!(scene.source_identity_count(), 2);
        assert_eq!(scene.profile_resource_count(), 2);
        let mut entries = scene.metadata_entries();
        let first = entries.next().unwrap();
        assert_eq!(
            first.owner(),
            MetadataOwnerRef::Entity(scene.metadata_owner())
        );
        assert_eq!(first.value(), MetadataValueRef::Bool(true));
        assert_eq!(
            entries.next().unwrap().value(),
            MetadataValueRef::I64(i64::MIN)
        );
        assert_eq!(
            entries.next().unwrap().value(),
            MetadataValueRef::U64(u64::MAX)
        );
        let MetadataValueRef::F64(value) = entries.next().unwrap().value() else {
            panic!()
        };
        assert_eq!(value.to_bits(), (-0.0_f64).to_bits());
        assert_eq!(
            entries.next().unwrap().value(),
            MetadataValueRef::String("text α")
        );
        let MetadataValueRef::Bytes {
            media_type,
            payload,
        } = entries.next().unwrap().value()
        else {
            panic!()
        };
        assert_eq!((media_type, payload), ("application/json", JSON));
        let MetadataValueRef::Vec2(values) = entries.next().unwrap().value() else {
            panic!()
        };
        assert_eq!(
            [values[0].to_bits(), values[1].to_bits()],
            [(-0.0_f32).to_bits(), 1]
        );
        assert_eq!(
            entries.next().unwrap().value(),
            MetadataValueRef::Vec3(&[-0.0, 1.0, 2.0])
        );
        assert_eq!(
            entries.next().unwrap().value(),
            MetadataValueRef::Vec4(&[0.0, -0.0, 2.0, 3.0])
        );
        assert_eq!(
            entries.next().unwrap().value(),
            MetadataValueRef::Color(&[0.0, -0.0, 0.5, 1.0])
        );
        assert!(
            matches!(entries.next().unwrap().value(), MetadataValueRef::Mat4(values) if values.len() == 16)
        );
        assert_eq!(
            entries.next().unwrap().value(),
            MetadataValueRef::DenseArray(MetadataArrayRef::Bool(&[0, 1]))
        );
        assert_eq!(
            entries.next().unwrap().value(),
            MetadataValueRef::DenseArray(MetadataArrayRef::I64(&[i64::MIN, i64::MAX]))
        );
        assert_eq!(
            entries.next().unwrap().value(),
            MetadataValueRef::DenseArray(MetadataArrayRef::U64(&[0, u64::MAX]))
        );
        let MetadataValueRef::DenseArray(MetadataArrayRef::F64(values)) =
            entries.next().unwrap().value()
        else {
            panic!()
        };
        assert_eq!(
            [values[0].to_bits(), values[1].to_bits()],
            [(-0.0_f64).to_bits(), PRECISE.to_bits()]
        );
        for entry in entries.by_ref().take(5) {
            let MetadataValueRef::DenseArray(array) = entry.value() else {
                panic!()
            };
            assert_eq!(array.len(), 1);
        }
        let node_entry = entries.next().unwrap();
        let node = scene.nodes().nth(1).unwrap();
        assert_eq!(
            node_entry.owner(),
            MetadataOwnerRef::Entity(node.metadata_owner())
        );
        assert_eq!(
            node.metadata_owner().descriptor(),
            SceneProfileOwner::Node(2)
        );
        for (index, entry) in entries.enumerate() {
            assert_eq!(entry.key(), "test:json");
            let MetadataOwnerRef::SourceObject(source) = entry.owner() else {
                panic!()
            };
            assert_eq!(source.index(), index);
            assert_eq!(source.generation(), scene.generation());
            assert_eq!(source.format(), "org.baozi.gltf");
            assert_eq!(source, scene.source_identities().nth(index).unwrap());
            let resource = source.resource();
            assert_eq!(resource.index(), index);
            assert_eq!(resource.generation(), scene.generation());
            assert_eq!(
                resource.stability(),
                ProfileResourceStabilityRef::SessionOpaque(ProfileSessionOpaqueReason::CallerLabel)
            );
            if index == 0 {
                assert_eq!(
                    source.locator(),
                    SourceObjectLocatorRef::Indexed {
                        kind: SourceObjectKind::Document,
                        index: u64::MAX
                    }
                );
                assert_eq!(source.display_name(), Some("Document"));
                assert_eq!(
                    source.content_digest(),
                    Some((DigestAlgorithm::Sha256, [7; 32].as_slice()))
                );
                assert_eq!(resource.identity(), "models/scene.glb");
            } else {
                assert_eq!(
                    source.locator(),
                    SourceObjectLocatorRef::Namespaced {
                        schema: "test:gltf-object",
                        schema_version: 2,
                        value: "accessors/7"
                    }
                );
                assert_eq!(source.display_name(), None);
                assert_eq!(source.content_digest(), None);
                assert_eq!(resource.identity(), "models/buffer.bin");
            }
        }
        assert_eq!(scene.metadata_entries().rev().count(), 23);
        assert_eq!(scene.profile_resources().rev().count(), 2);
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    let snapshot = scene
        .capture_snapshot(SceneSnapshotLimits::try_new(32768, 256, 1_000_000).unwrap())
        .unwrap();
    assert!(snapshot.preview().contains("owner_kind=19"));

    // The borrowed fields outlive a temporary entry and iterator, while copied fields outlive Scene.
    let source = scene.source_identities().nth(1).unwrap();
    let SourceObjectLocatorRef::Namespaced { schema, value, .. } = source.locator() else {
        panic!()
    };
    let owned_locator = (
        schema.to_owned(),
        value.to_owned(),
        source.resource().identity().to_owned(),
    );
    let MetadataValueRef::Bytes {
        media_type,
        payload,
    } = scene.metadata_entries().nth(21).unwrap().value()
    else {
        panic!()
    };
    let owned_json = (media_type.to_owned(), payload.to_vec());
    drop(scene);
    drop(parts.report);
    assert_eq!(
        owned_locator,
        (
            "test:gltf-object".to_owned(),
            "accessors/7".to_owned(),
            "models/buffer.bin".to_owned()
        )
    );
    assert_eq!(owned_json, ("application/json".to_owned(), JSON.to_vec()));
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn metadata_entity_tokens_do_not_alias_another_scene_generation() {
    let factory = test_factory();
    let account = test_account(&factory, 4096);
    let first = build_metadata_scene(&account);
    let second = build_metadata_scene(&account);
    let first_scene = ValidatedScene::from_node_scene(first.scene);
    let second_scene = ValidatedScene::from_node_scene(second.scene);
    assert_ne!(first_scene.metadata_owner(), second_scene.metadata_owner());
    assert_ne!(
        first_scene.nodes().next().unwrap().metadata_owner(),
        second_scene.nodes().next().unwrap().metadata_owner()
    );
    assert_ne!(
        first_scene.source_identities().next().unwrap(),
        second_scene.source_identities().next().unwrap()
    );
    drop(first_scene);
    drop(second_scene);
    drop(first.report);
    drop(second.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}
