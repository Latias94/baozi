use baozi_operation::{
    CancellationToken, OperationControl, PhysicalStorageAccount, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
};

use crate::FormatId;
use crate::SceneValidationLimits;
use crate::appearance::{AlphaMode, MaterialFactors, MaterialSemanticsRef, ShadingModel};
use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::construction::node_scene::semantic::{
    NodeSceneProfileCursor, NodeSceneProfileMetadataArray, NodeSceneProfileMetadataArrayKind,
    NodeSceneProfileMetadataValue, NodeSceneProfileStep,
};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneConstructionTerminal, NodeSceneSuccessParts, ValidatedNodeScene,
};
use crate::canonical_scene::store::construction::{
    SceneConstructionLimits, SceneConstructionOperation,
};
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AuthoringRange, CameraAuthoring,
    CameraLocalId, CanonicalProfileOwner, DeformationAuthoringBatch, GeometryAttributeAuthoring,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, LightAuthoring, LightLocalId,
    LineageAuthoringBatch, LogicalMeshLocalId, LogicalMorphAuthoring, MaterialAuthoring,
    MaterialLocalId, MeshInstanceDeformationAuthoring, MetadataAuthoringBatch,
    MetadataAuthoringEntry, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent,
    ObservedResourceAuthoring, OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring,
    ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring,
    ProvenanceOperationAuthoring, ResourceIdentityLocalId, SceneAuthoringBatch,
    SessionOpaqueReason, SourceIdentityAuthoring, SourceIdentityLocalId, SpatialAuthoringBatch,
};
use crate::canonical_scene::store::schema::profile::profile_segment_field;
use crate::entity::{AuthoredTransform, CameraProjection, LightEmission, LightKind};
use crate::extension::{
    ExtensionArrayRef, ExtensionMediaType, ExtensionSchemaId, ExtensionValueRef,
};
use crate::identity::NodeId;
use crate::math::{Color, Mat4, Quat, Vec2, Vec3, Vec4};
use crate::namespaced::NamespacedName;
use crate::provenance::{
    OperationKind, OptionsDigest, ResourceIdentity, SourceObjectKind, SourceObjectLocator,
};
use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, SceneSpace, TangentBasisConvention,
    TangentHandednessEncoding, TransformConvention, UvOrigin,
};

use super::super::edit::{
    NodeTransformEditStart, NodeTransformEditTerminal, try_begin_node_transform_edit,
};
use super::super::tests::profile_fixture;
use super::{
    MAX_COLLECTION_ITEMS, MAX_PREVIEW_BYTES, MAX_WORK, NodeSceneSnapshot,
    NodeSceneSnapshotAlgorithm, NodeSceneSnapshotError, NodeSceneSnapshotLimit,
    NodeSceneSnapshotLimits, SnapshotWorkMeter, capture_node_scene_with_test_transcript,
};

const KNOWN_SINGLE_TRS_DIGEST: [u8; 32] = [
    0x66, 0x2f, 0x11, 0x9f, 0x56, 0x4f, 0xe1, 0x52, 0x61, 0xe4, 0x76, 0xe9, 0x29, 0x71, 0x7a, 0x74,
    0xc8, 0xd0, 0xea, 0x61, 0xa9, 0x13, 0xf8, 0x38, 0x2d, 0x82, 0x21, 0x0c, 0xd9, 0x28, 0x14, 0x28,
];
const KNOWN_GENERIC_METADATA_DIGEST: [u8; 32] = [
    0x4b, 0xa1, 0x44, 0xc1, 0x99, 0x93, 0xa2, 0xdb, 0x94, 0xa0, 0x0a, 0x78, 0x4d, 0x3e, 0xde, 0x07,
    0xd5, 0x27, 0xc4, 0xdb, 0xc8, 0x37, 0xca, 0x27, 0x5d, 0x72, 0xef, 0xcd, 0x21, 0xf3, 0x74, 0x37,
];
// Known vectors are factored at canonical domain and owner-row boundaries, but every
// assertion still compares the complete transcript byte for byte.
const KNOWN_SINGLE_TRS_TRANSCRIPT_PREFIX_HEX: &str = concat!(
    "011e0000000000000062616f7a692e63616e6f6e6963616c2d7363656e652e736e617073686f740201000000030600",
    "000000000000736861323536a60c01a7a800a900aa00ab0c00ac00ad00ae00af00b200b30c010b0000000000000000",
    "ba0c010b0000000000000000040c01080d00000000000000000b0100000000000000050c01060d0000000000000000",
    "0a020e000000000e000000000e000000000e000000000e000000000e000000000e0000803f0e0000803f0e0000803f",
    "0e0000803f0b0a00000000000000c40c00c50c00070c010b00000000000000000b0100000000000000",
);
const KNOWN_MATRIX_CHILD_DIGEST: [u8; 32] = [
    0x9c, 0x7d, 0x3b, 0x38, 0xb0, 0xdb, 0xc5, 0xa6, 0x2a, 0xe1, 0x97, 0x8c, 0xcd, 0xd4, 0xf0, 0x7c,
    0x38, 0xd1, 0xbe, 0x4c, 0x94, 0xeb, 0xb6, 0xd1, 0xbd, 0xaf, 0x79, 0xad, 0xf2, 0x1f, 0xd9, 0xdc,
];
const KNOWN_SPATIAL_DIGEST: [u8; 32] = [
    0x73, 0x4a, 0xb1, 0x8b, 0x33, 0x51, 0x4a, 0x08, 0xd4, 0x51, 0xf5, 0x25, 0xb4, 0xb2, 0x40, 0xd0,
    0xa4, 0x3f, 0x40, 0x12, 0xb8, 0xcf, 0x7e, 0x55, 0x34, 0x8f, 0xc8, 0xea, 0xe6, 0x56, 0x53, 0x92,
];
const KNOWN_SPATIAL_TRANSCRIPT_PREFIX_HEX: &str = concat!(
    "011e0000000000000062616f7a692e63616e6f6e6963616c2d7363656e652e736e617073686f74020100000003060000000000000073",
    "6861323536a60c01a7a802a904aa05ab0c01c67b14ae47e17a843fac01ad02ae01af02b015000000000000006578616d706c653a7461",
    "6e67656e742d6261736973b102000000b202b014000000000000006578616d706c653a74616e67656e742d7369676eb103000000b30c",
    "01b40d0000000000000000b500b6c70000803fb70c01c70000c03fb8c7cdcccc3db90c01c70000c8420b0100000000000000ba0c01bb",
    "0d0000000000000000bc02bd0c01c700004842be0c01c70000803ebf0c01c70000403fc0c70000803fc1c70000803fc2c70000803fc3",
    "c7000020410b0100000000000000040c01080d00000000000000000b0100000000000000050c01060d00000000000000000a020e0000",
    "00000e000000000e000000000e000000000e000000000e000000000e0000803f0e0000803f0e0000803f0e0000803f0b0a0000000000",
    "0000c40c010d0000000000000000c50c010d0000000000000000070c010b00000000000000000b0100000000000000",
);
const KNOWN_MATRIX_CHILD_TRANSCRIPT_PREFIX_HEX: &str = concat!(
    "011e0000000000000062616f7a692e63616e6f6e6963616c2d7363656e652e736e617073686f74020100000003",
    "0600000000000000736861323536a60c01a7a800a900aa00ab0c00ac00ad00ae00af00b200b30c01",
    "0b0000000000000000ba0c010b0000000000000000040c01080d00000000000000000b0100000000000000050c01060d00000000",
    "000000000a010e0000803f0e000000000e000000000e000000000e000000000e0000803f0e000000000e000000",
    "000e000000000e000000000e0000803f0e000000000e000080400e000000000e000000000e0000803f0b100000",
    "0000000000c40c00c50c00070c01090d01000000000000000b0100000000000000060d01000000000000000a020e000000410e",
    "000000000e000000000e000000000e000000000e000000000e0000803f0e0000803f0e0000803f0e0000803f0b",
    "0a00000000000000c40c00c50c00070c010b00000000000000000b0200000000000000",
);
const EMPTY_PRE_PROFILE_DOMAINS_TRANSCRIPT_HEX: &str = concat!(
    "330c01340c010b0000000000000000380c010b0000000000000000400c010b000000000000000010",
    "0c010b0000000000000000440c010b00000000000000000f0c01120c010b0000000000000000140c",
    "010b0000000000000000190c010b0000000000000000280c010b00000000000000004a0c014b0c01",
    "0b00000000000000005d0c010b0000000000000000610c010b0000000000000000660c010b000000",
    "00000000006e0c010b0000000000000000710c010b0000000000000000280c010b00000000000000",
    "00780c01790c010b00000000000000007f0c010b0000000000000000",
);
const EMPTY_SCENE_TRANSCRIPT_PREFIX_HEX: &str = concat!(
    "011e0000000000000062616f7a692e63616e6f6e6963616c2d7363656e652e736e617073686f740201000000030600",
    "000000000000736861323536a60c01a7a800a900aa00ab0c00ac00ad00ae00af00b200b30c010b0000000000000000",
    "ba0c010b0000000000000000040c010b0000000000000000050c010b0000000000000000",
);
const FRESH_PROFILE_TRANSCRIPT_PREFIX_HEX: &str = concat!(
    "c8ce01cb00ce00cf0000000000000000cb01ce00cf0000000000000000cb02ce00cf0000000000000000",
    "cb03ce00cf0000000000000000cb04ce00cf0000000000000000cb05ce00cf0000000000000000",
    "cb06ce00cf0000000000000000c9cc01ce01cf0100000000000000cacc01cd0000000000000000",
    "cb00ce01d6190000000000000066697874757265732f6e6f64652d7363656e652f696e707574cb01d001cb05d000",
    "c9cc02ce01cf0100000000000000cacc02cd0000000000000000cb00d20000000000000000",
    "cb01ce01d611000000000000006f72672e62616f7a692e66697874757265cb02d000cb03d000",
    "cb04d20000000000000000cb08ce00cf0000000000000000cb09ce00cf0000000000000000",
    "c9cc03ce01cf0100000000000000cacc03cd0000000000000000cb00d20100000000000000",
    "cb01d20000000000000000cb02ce01cf0000000000000000cb03d000cb04d20000000000000000",
    "c9cc04ce01cf0000000000000000",
);
const PROFILE_OWNER_COLLECTION_2_HEX: &str = "c9cc05ce01cf0200000000000000";
const PROFILE_OWNER_COLLECTION_3_HEX: &str = "c9cc05ce01cf0300000000000000";
const PROFILE_OWNER_COLLECTION_4_HEX: &str = "c9cc05ce01cf0400000000000000";
const PROFILE_OWNER_COLLECTION_5_HEX: &str = "c9cc05ce01cf0500000000000000";
const PROFILE_OWNER_COLLECTION_8_HEX: &str = "c9cc05ce01cf0800000000000000";
const PROFILE_SCENE_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0000000000000000cb00d000cb01d100000000cb02d20000000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_SYNTHETIC_ROOT_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0100000000000000cb00d001cb01d100000000cb02d20100000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_NODE_0_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0200000000000000cb00d002cb01d101000000cb02d20200000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_NODE_1_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0300000000000000cb00d002cb01d102000000cb02d20300000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_CAMERA_0_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0300000000000000cb00d003cb01d100000000cb02d20300000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_LIGHT_0_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0400000000000000cb00d004cb01d100000000cb02d20400000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_MATERIAL_0_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0300000000000000cb00d009cb01d100000000cb02d20300000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_LOGICAL_MESH_0_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0400000000000000cb00d00acb01d100000000cb02d20400000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_PRIMITIVE_0_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0500000000000000cb00d00bcb01d100000000cb02d20500000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_ATTRIBUTE_0_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0600000000000000cb00d00ccb01d100000000cb02d20600000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const PROFILE_MESH_INSTANCE_0_OWNER_ROW_HEX: &str = concat!(
    "cacc05cd0700000000000000cb00d00dcb01d100000000cb02d20700000000000000",
    "cb03ce01cf0100000000000000d700000000",
);
const EMPTY_PROFILE_METADATA_TRANSCRIPT_HEX: &str = "c9cc06ce00cf0000000000000000";
const KNOWN_POINT_GEOMETRY_DIGEST: [u8; 32] = [
    0xb6, 0x17, 0x73, 0x7a, 0x79, 0x37, 0x9f, 0xc6, 0xc4, 0xd9, 0xc5, 0x7d, 0xff, 0x92, 0x57, 0xcf,
    0xb3, 0x31, 0xff, 0x13, 0xd7, 0x52, 0x4e, 0x3a, 0xff, 0x83, 0x25, 0xf3, 0x02, 0xfd, 0xb5, 0x89,
];
const KNOWN_POINT_GEOMETRY_TRANSCRIPT_HEX: &str = concat!(
    "011e0000000000000062616f7a692e63616e6f6e6963616c2d7363656e652e736e617073686f74020100000003060000",
    "0000000000736861323536a60c01a7a800a900aa00ab0c00ac00ad00ae00af00b200b30c010b0000000000000000",
    "ba0c010b0000000000000000040c01080d00000000000000000b0100000000000000050c01060d00000000000000000a02",
    "0e000000000e000000000e000000000e000000000e000000000e000000000e0000803f0e0000803f0e0000803f0e0000",
    "803f0b0a00000000000000c40c00c50c00070c010b00000000000000000b0100000000000000330c01340c010b000000000000000038",
    "0c010b0000000000000000400c010b0000000000000000100c01110d00000000000000008e008f009000000000910092",
    "00930094004800000000000000000b0000000000000000950000000000000000960b2700000000000000a500000000a5",
    "00000000a500000000a500000000a500000000a500000000a500000000a500000000a500000000a500000000a5000000",
    "00a500000000a500000000a500000000a500000000a500000000a500000000a500000000a500000000a500000000a500",
    "000000a500000000a500000000a500000000a500000000a500000000a500000000a500000000a500000000a500000000",
    "a500000000a500000000a500000000a500000000a500000000a500000000a500000000a500000000a5000000000b0100",
    "000000000000440c010b00000000000000000f0c01120c01130d00000000000000002c00000000000000000b01000000",
    "000000000b0100000000000000140c01150d00000000000000002e002f0100000030010000003101000000160c000b00",
    "00000000000000170c000b0000000000000000180c010d0000000000000000192d00000000000000000b010000000000",
    "00000b0100000000000000190c011a0d00000000000000001b001c000000001d001e001f032000210022010000002301",
    "000000240300000000000000250c010e0000803f0e000000400e000040400b0300000000000000260c000b0000000000",
    "000000270c000b00000000000000000b0100000000000000280c01290d00000000000000002a00000000000000002b00",
    "000000000000000b01000000000000004a0c014b0c010b00000000000000005d0c015e0d00000000000000005f000000",
    "00000000000b0000000000000000600c010b00000000000000000b0100000000000000610c010b000000000000000066",
    "0c01670d0000000000000000680c000b00000000000000006e0c000d00000000000000000b00000000000000000b0100",
    "0000000000006e0c010b0000000000000000710c010b0000000000000000280c01290d00000000000000004c0c00600c",
    "000b00000000000000000b0100000000000000780c01790c010b00000000000000007f0c010b0000000000000000",
);

fn operation_limits() -> SceneConstructionLimits {
    SceneConstructionLimits {
        operation_live_bytes: u64::MAX,
        dimension_live_bytes: u64::MAX,
        attempted_work: u64::MAX,
        consumed_work: u64::MAX,
        committed_output: u64::MAX,
        published_evidence: u64::MAX,
    }
}

fn snapshot_limits(bytes: usize, items: usize) -> NodeSceneSnapshotLimits {
    NodeSceneSnapshotLimits::try_new(bytes, items, MAX_WORK).expect("valid snapshot limits")
}

fn capture_node_scene(
    scene: &ValidatedNodeScene,
    limits: NodeSceneSnapshotLimits,
    cancellation: Option<&CancellationToken>,
) -> Result<NodeSceneSnapshot, NodeSceneSnapshotError> {
    scene.capture_snapshot(limits, cancellation)
}

fn inspect_single_node_profile(
    scene: &ValidatedNodeScene,
    inspect: impl FnOnce(&mut NodeSceneProfileCursor<'_>),
) {
    let view = scene.semantic_view().expect("metadata semantic view");
    let mut cursor = view.into_cursor().expect("metadata semantic cursor");
    let mut roots = cursor.take_roots().expect("metadata roots");
    assert_eq!(roots.next().expect("metadata root"), Some(0));
    assert_eq!(roots.next().expect("metadata root end"), None);
    roots.finish().expect("complete metadata roots");
    let node = cursor
        .next_node()
        .expect("metadata node read")
        .expect("metadata node");
    node.into_children()
        .finish()
        .expect("complete metadata node children");
    assert!(cursor.next_node().expect("metadata node end").is_none());

    {
        let appearance = cursor.appearance().expect("metadata appearance");
        assert!(appearance.next_image().expect("image end").is_none());
        assert!(appearance.next_sampler().expect("sampler end").is_none());
        assert!(appearance.next_texture().expect("texture end").is_none());
        assert!(appearance.next_material().expect("material end").is_none());
        assert!(appearance.next_binding().expect("binding end").is_none());
    }
    {
        let geometry = cursor.geometry().expect("metadata geometry");
        assert!(
            geometry
                .next_logical_mesh()
                .expect("logical mesh end")
                .is_none()
        );
        assert!(geometry.next_primitive().expect("primitive end").is_none());
        assert!(geometry.next_attribute().expect("attribute end").is_none());
        assert!(
            geometry
                .next_mesh_instance()
                .expect("mesh instance end")
                .is_none()
        );
    }
    {
        let deformation = cursor.deformation().expect("metadata deformation");
        assert!(deformation.next_skin().expect("skin end").is_none());
        assert!(
            deformation
                .next_logical_morph()
                .expect("logical morph end")
                .is_none()
        );
        assert!(
            deformation
                .next_morph_definition()
                .expect("morph definition end")
                .is_none()
        );
        assert!(
            deformation
                .next_primitive()
                .expect("deformation primitive end")
                .is_none()
        );
        assert!(
            deformation
                .next_morph_target()
                .expect("morph target end")
                .is_none()
        );
        assert!(
            deformation
                .next_morph_delta()
                .expect("morph delta end")
                .is_none()
        );
        assert!(
            deformation
                .next_mesh_instance()
                .expect("deformation mesh instance end")
                .is_none()
        );
    }
    {
        let animation = cursor.animation().expect("metadata animation");
        assert!(animation.next_clip().expect("animation clip end").is_none());
        assert!(
            animation
                .next_channel()
                .expect("animation channel end")
                .is_none()
        );
    }

    inspect(cursor.profile().expect("metadata profile"));
    cursor
        .finish()
        .expect("complete metadata semantic traversal");
}

fn factory() -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 16)
            .expect("snapshot test factory limits"),
    )
    .expect("snapshot test factory")
}

fn account(factory: &PhysicalStorageAccountFactory) -> PhysicalStorageAccount {
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, 512).expect("snapshot test account limits"),
        )
        .expect("snapshot test account")
}

fn build_scene(
    account: &PhysicalStorageAccount,
    roots: &[NodeLocalId],
    nodes: &[NodeAuthoringNode],
) -> NodeSceneSuccessParts {
    build_scene_with_geometry(account, roots, nodes, GeometryAuthoringBatch::empty())
}

fn build_scene_with_geometry<'input>(
    account: &PhysicalStorageAccount,
    roots: &'input [NodeLocalId],
    nodes: &'input [NodeAuthoringNode],
    geometry: GeometryAuthoringBatch<'input>,
) -> NodeSceneSuccessParts {
    build_scene_with_appearance_and_geometry(
        account,
        roots,
        nodes,
        AppearanceAuthoringBatch::empty(),
        geometry,
    )
}

fn build_scene_with_appearance_and_geometry<'input>(
    account: &PhysicalStorageAccount,
    roots: &'input [NodeLocalId],
    nodes: &'input [NodeAuthoringNode],
    appearance: AppearanceAuthoringBatch<'input>,
    geometry: GeometryAuthoringBatch<'input>,
) -> NodeSceneSuccessParts {
    build_scene_with_spatial_appearance_and_geometry(
        account,
        roots,
        nodes,
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        appearance,
        geometry,
    )
}

fn build_scene_with_spatial_appearance_and_geometry<'input>(
    account: &PhysicalStorageAccount,
    roots: &'input [NodeLocalId],
    nodes: &'input [NodeAuthoringNode],
    spatial: SpatialAuthoringBatch<'input>,
    appearance: AppearanceAuthoringBatch<'input>,
    geometry: GeometryAuthoringBatch<'input>,
) -> NodeSceneSuccessParts {
    build_scene_with_spatial_appearance_geometry_and_metadata(
        account,
        roots,
        nodes,
        spatial,
        appearance,
        geometry,
        MetadataAuthoringBatch::empty(),
    )
}

fn build_scene_with_metadata<'input>(
    account: &PhysicalStorageAccount,
    roots: &'input [NodeLocalId],
    nodes: &'input [NodeAuthoringNode],
    metadata: MetadataAuthoringBatch<'input>,
) -> NodeSceneSuccessParts {
    build_scene_with_spatial_appearance_geometry_and_metadata(
        account,
        roots,
        nodes,
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        metadata,
    )
}

fn build_scene_with_spatial_appearance_geometry_and_metadata<'input>(
    account: &PhysicalStorageAccount,
    roots: &'input [NodeLocalId],
    nodes: &'input [NodeAuthoringNode],
    spatial: SpatialAuthoringBatch<'input>,
    appearance: AppearanceAuthoringBatch<'input>,
    geometry: GeometryAuthoringBatch<'input>,
    metadata: MetadataAuthoringBatch<'input>,
) -> NodeSceneSuccessParts {
    let logical_morphs = vec![
        LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[]);
        geometry.logical_meshes().len()
    ];
    let primitive_deformations = vec![
        PrimitiveDeformationAuthoring::new(Presence::Absent, &[]);
        geometry.primitives().len()
    ];
    let mesh_instance_deformations =
        vec![
            MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent);
            geometry.mesh_instances().len()
        ];
    let deformation = DeformationAuthoringBatch::new(
        &[],
        &[],
        &logical_morphs,
        &primitive_deformations,
        &mesh_instance_deformations,
    );
    let node_batch = NodeAuthoringBatch::new(Presence::Present(roots), nodes);
    let animations = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile.batch_with_metadata(metadata),
    );
    publish_scene_input(account, input)
}

fn publish_scene_input(
    account: &PhysicalStorageAccount,
    input: SceneAuthoringBatch<'_>,
) -> NodeSceneSuccessParts {
    let prepared = SceneConstructionOperation::with_account_for_scene_input(
        account,
        operation_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .expect("snapshot Scene input admission")
    .try_validate()
    .unwrap_or_else(|rejected| {
        panic!(
            "snapshot input must pass authoring validation: {:?}",
            rejected.failure()
        )
    })
    .prepare()
    .expect("snapshot Scene must prepare");
    match prepared.handoff() {
        NodeSceneConstructionTerminal::Success(parts) => parts,
        NodeSceneConstructionTerminal::RichFailure(_) => {
            panic!("valid snapshot Scene published rich failure")
        }
        NodeSceneConstructionTerminal::MinimalFallback(_) => {
            panic!("valid snapshot Scene published minimal fallback")
        }
    }
}

fn parent(node: u32, ordinal: u32) -> Presence<NodeParent> {
    Presence::Present(NodeParent::new(NodeLocalId::new(node), ordinal))
}

fn trs(translation_x: f32) -> AuthoredTransform {
    AuthoredTransform::try_trs(
        Vec3::new(translation_x, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    )
    .expect("valid authored TRS")
}

fn matrix(translation_x: f32) -> AuthoredTransform {
    AuthoredTransform::try_matrix(Mat4 {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [translation_x, 0.0, 0.0, 1.0],
        ],
    })
    .expect("valid authored matrix")
}

fn encode_hex(actual: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(actual.len().saturating_mul(2));
    for byte in actual.iter().copied() {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn assert_hex_parts(actual: &[u8], expected: &[&str]) {
    let encoded = encode_hex(actual);
    let expected = expected.concat();
    assert_eq!(encoded, expected);
}

fn extension_schema(name: &str, version: u32) -> ExtensionSchemaId {
    ExtensionSchemaId::new(name, version).expect("canonical spatial test schema")
}

fn namespaced_scene_space() -> SceneSpace {
    SceneSpace {
        handedness: Handedness::Left,
        up_axis: Axis::NegativeY,
        front_axis: Axis::PositiveZ,
        unit_scale_to_meters: Some(0.01),
        front_face_winding: FrontFaceWinding::Clockwise,
        uv_origin: UvOrigin::LowerLeft,
        transform_convention: TransformConvention::ColumnVectorLocalToParent,
        tangent_basis: TangentBasisConvention::try_namespaced(Some(extension_schema(
            "example:tangent-basis",
            2,
        )))
        .expect("namespaced tangent-basis convention"),
        tangent_handedness_encoding: TangentHandednessEncoding::try_namespaced(Some(
            extension_schema("example:tangent-sign", 3),
        ))
        .expect("namespaced tangent-encoding convention"),
    }
}

fn white_emission(intensity: f32) -> LightEmission {
    LightEmission::try_from_scaled_linear_srgb(Color::WHITE, intensity)
        .expect("canonical white spatial test emission")
}

fn capture_spatial_scene(
    account: &PhysicalStorageAccount,
    roots: &[NodeLocalId],
    nodes: &[NodeAuthoringNode],
    scene_space: &SceneSpace,
    cameras: &[CameraAuthoring],
    lights: &[LightAuthoring],
) -> NodeSceneSnapshot {
    let parts = build_scene_with_spatial_appearance_and_geometry(
        account,
        roots,
        nodes,
        SpatialAuthoringBatch::new(scene_space, cameras, lights),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
    );
    capture_node_scene(&parts.scene, snapshot_limits(16_384, 64), None)
        .expect("canonical spatial test snapshot")
}

fn capture_single_spatial_digest(
    account: &PhysicalStorageAccount,
    scene_space: &SceneSpace,
    projection: CameraProjection,
    light_kind: LightKind,
    emission: LightEmission,
) -> [u8; 32] {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Present(CameraLocalId::new(0)),
        Presence::Present(LightLocalId::new(0)),
    )];
    let cameras = [CameraAuthoring::new(projection)];
    let lights = [LightAuthoring::new(light_kind, emission)];
    capture_spatial_scene(account, &roots, &nodes, scene_space, &cameras, &lights).digest
}

fn assert_digest_mutation(label: &str, baseline: [u8; 32], mutation: [u8; 32]) {
    assert_ne!(baseline, mutation, "{label} was absent from the digest");
}

fn single_root_scene(
    account: &PhysicalStorageAccount,
    transform: AuthoredTransform,
) -> NodeSceneSuccessParts {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        transform,
        Presence::Absent,
        Presence::Absent,
    )];
    build_scene(account, &roots, &nodes)
}

fn point_geometry_scene(
    account: &PhysicalStorageAccount,
    positions: &[f32],
) -> NodeSceneSuccessParts {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Present(MaterialLocalId::new(0)),
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    let shading_model = ShadingModel::Unlit;
    let alpha_mode = AlphaMode::Opaque;
    let material_semantics =
        MaterialSemanticsRef::new(&shading_model, &alpha_mode, false, MaterialFactors::empty());
    let materials = [MaterialAuthoring::new(
        AuthoringRange::new(0, 0),
        material_semantics,
    )];
    build_scene_with_appearance_and_geometry(
        account,
        &roots,
        &nodes,
        AppearanceAuthoringBatch::new(&[], &[], &[], &[], &materials),
        GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &instances),
    )
}

fn point_index_presence_scene(
    account: &PhysicalStorageAccount,
    indices_present: bool,
) -> NodeSceneSuccessParts {
    let roots = [];
    let nodes = [];
    let positions = [0.0_f32, 0.0, 0.0];
    let empty_indices = [];
    let indices: Presence<&[u32]> = if indices_present {
        Presence::Present(&empty_indices)
    } else {
        Presence::Absent
    };
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        indices,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    build_scene_with_geometry(
        account,
        &roots,
        &nodes,
        GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]),
    )
}

fn corner_normal_scene<'input>(
    account: &PhysicalStorageAccount,
    normal_values: &'input [f32],
    normal_source_indices: Presence<&'input [u32]>,
) -> NodeSceneSuccessParts {
    let roots = [];
    let nodes = [];
    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
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
            AttributeDomain::Corner,
            3,
            normal_values,
            normal_source_indices,
        ),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    build_scene_with_geometry(
        account,
        &roots,
        &nodes,
        GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]),
    )
}

fn all_attribute_payload_kinds_scene(account: &PhysicalStorageAccount) -> NodeSceneSuccessParts {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    let triangle_indices = [0_u32, 1, 2];
    let polygon_boundaries = [3_u32];
    let first_positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let second_positions = [0.0_f32, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0];
    let third_positions = [0.0_f32, 0.0, 2.0, 1.0, 0.0, 2.0, 0.0, 1.0, 2.0];
    let sparse_indices = [0_u32];
    let sparse_u32_values = [7_u32];
    let sparse_u32_default = [0_u32];
    let sparse_f32_values = [0.25_f32, 0.5, 0.75];
    let sparse_f32_default = [0.0_f32, 0.0, 0.0];
    let dense_u32_values = [3_u32];
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &first_positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_u32(
            AttributeSemantic::SmoothingGroup,
            AttributeDomain::Face,
            1,
            1,
            &sparse_indices,
            &sparse_u32_values,
            &sparse_u32_default,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &second_positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Color(0),
            AttributeDomain::Face,
            3,
            1,
            &sparse_indices,
            &sparse_f32_values,
            &sparse_f32_default,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &third_positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::u32(
            AttributeSemantic::SmoothingGroup,
            AttributeDomain::Face,
            1,
            &dense_u32_values,
        ),
    ];
    let primitives = [
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&triangle_indices),
            Presence::Present(MaterialLocalId::new(0)),
            AuthoringRange::new(0, 2),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Polygons(&polygon_boundaries),
            Presence::Present(&triangle_indices),
            Presence::Absent,
            AuthoringRange::new(2, 2),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(4, 2),
        ),
    ];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 3))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    let shading_model = ShadingModel::Unlit;
    let alpha_mode = AlphaMode::Opaque;
    let material_semantics =
        MaterialSemanticsRef::new(&shading_model, &alpha_mode, false, MaterialFactors::empty());
    let materials = [MaterialAuthoring::new(
        AuthoringRange::new(0, 0),
        material_semantics,
    )];
    build_scene_with_appearance_and_geometry(
        account,
        &roots,
        &nodes,
        AppearanceAuthoringBatch::new(&[], &[], &[], &[], &materials),
        GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &instances),
    )
}

#[test]
fn known_vector_is_stable_and_human_reviewable() {
    let factory = factory();
    let account = account(&factory);
    let parts = single_root_scene(&account, AuthoredTransform::identity_trs());

    let (snapshot, transcript) =
        capture_node_scene_with_test_transcript(&parts.scene, snapshot_limits(4_096, 64), None)
            .expect("known Scene snapshot");

    assert_eq!(snapshot.domain, "baozi.canonical-scene.snapshot");
    assert_eq!(snapshot.schema_revision, 1);
    assert_eq!(snapshot.algorithm, NodeSceneSnapshotAlgorithm::Sha256);
    assert_eq!(snapshot.digest, KNOWN_SINGLE_TRS_DIGEST);
    assert!(transcript.complete);
    assert_hex_parts(
        &transcript.bytes,
        &[
            KNOWN_SINGLE_TRS_TRANSCRIPT_PREFIX_HEX,
            EMPTY_PRE_PROFILE_DOMAINS_TRANSCRIPT_HEX,
            FRESH_PROFILE_TRANSCRIPT_PREFIX_HEX,
            PROFILE_OWNER_COLLECTION_3_HEX,
            PROFILE_SCENE_OWNER_ROW_HEX,
            PROFILE_SYNTHETIC_ROOT_OWNER_ROW_HEX,
            PROFILE_NODE_0_OWNER_ROW_HEX,
            EMPTY_PROFILE_METADATA_TRANSCRIPT_HEX,
        ],
    );
    assert_eq!(snapshot.totals.node_count, 1);
    assert_eq!(snapshot.totals.root_count, 1);
    assert_eq!(snapshot.totals.edge_count, 0);
    assert_eq!(snapshot.totals.image_count, 0);
    assert_eq!(snapshot.totals.sampler_count, 0);
    assert_eq!(snapshot.totals.texture_count, 0);
    assert_eq!(snapshot.totals.material_count, 0);
    assert_eq!(snapshot.totals.material_binding_count, 0);
    assert_eq!(snapshot.totals.logical_mesh_count, 0);
    assert_eq!(snapshot.totals.primitive_count, 0);
    assert_eq!(snapshot.totals.attribute_count, 0);
    assert_eq!(snapshot.totals.mesh_instance_count, 0);
    assert_eq!(snapshot.totals.skin_count, 0);
    assert_eq!(snapshot.totals.logical_morph_count, 0);
    assert_eq!(snapshot.totals.morph_definition_count, 0);
    assert_eq!(snapshot.totals.primitive_deformation_count, 0);
    assert_eq!(snapshot.totals.morph_target_count, 0);
    assert_eq!(snapshot.totals.morph_delta_count, 0);
    assert_eq!(snapshot.totals.deformation_mesh_instance_count, 0);
    assert_eq!(snapshot.totals.animation_clip_count, 0);
    assert_eq!(snapshot.totals.animation_channel_count, 0);
    assert_eq!(snapshot.totals.profile_resource_count, 1);
    assert_eq!(snapshot.totals.profile_source_count, 1);
    assert_eq!(snapshot.totals.profile_provenance_node_count, 1);
    assert_eq!(snapshot.totals.profile_operation_count, 0);
    assert_eq!(snapshot.totals.profile_evidence_count, 0);
    assert_eq!(snapshot.totals.profile_parent_reference_count, 0);
    assert_eq!(snapshot.totals.profile_owner_count, 3);
    assert_eq!(snapshot.totals.profile_attachment_count, 3);
    assert_eq!(snapshot.totals.profile_metadata_entry_count, 0);
    assert_eq!(snapshot.totals.transcript_bytes, 1_009);
    assert_eq!(snapshot.totals.work_units, 109);
    assert!(snapshot.preview.text.contains("roots(present,total=1)=[0]"));
    assert!(snapshot.preview.text.contains("transform=trs(total=10)"));
    assert!(
        snapshot
            .preview
            .text
            .contains("children(present)=[];total=0")
    );
    assert!(snapshot.preview.text.contains("geometry(present)"));
    assert!(snapshot.preview.text.contains("profile(present)"));
    assert!(!snapshot.preview.bytes_truncated);
    assert!(!snapshot.preview.items_truncated);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn generic_metadata_round_trips_every_value_kind_without_provenance_ids() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    let keys = [
        "test:a_bool",
        "test:b_i64",
        "test:c_u64",
        "test:d_f64",
        "test:e_string",
        "test:f_bytes",
        "test:g_vec2",
        "test:h_vec3",
        "test:i_vec4",
        "test:j_color",
        "test:k_mat4",
        "test:l_dense_empty",
    ]
    .map(|key| NamespacedName::new(key).expect("canonical metadata key"));
    let media_type =
        ExtensionMediaType::new("application/octet-stream").expect("canonical metadata media type");
    let byte_payload = [0_u8, 1, 2, 255];
    let empty_vec3 = [];
    let metadata = [
        MetadataAuthoringEntry::new(ProfileOwner::Scene, &keys[0], ExtensionValueRef::Bool(true)),
        MetadataAuthoringEntry::new(ProfileOwner::Scene, &keys[1], ExtensionValueRef::I64(-7)),
        MetadataAuthoringEntry::new(ProfileOwner::Scene, &keys[2], ExtensionValueRef::U64(9)),
        MetadataAuthoringEntry::new(ProfileOwner::Scene, &keys[3], ExtensionValueRef::F64(2.5)),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &keys[4],
            ExtensionValueRef::String("metadata"),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &keys[5],
            ExtensionValueRef::Bytes {
                media_type: &media_type,
                payload: &byte_payload,
            },
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &keys[6],
            ExtensionValueRef::Vec2(Vec2::new(1.0, 2.0)),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &keys[7],
            ExtensionValueRef::Vec3(Vec3::new(3.0, 4.0, 5.0)),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &keys[8],
            ExtensionValueRef::Vec4(Vec4::new(6.0, 7.0, 8.0, 9.0)),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &keys[9],
            ExtensionValueRef::Color(Color::linear_rgba(0.1, 0.2, 0.3, 0.4)),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &keys[10],
            ExtensionValueRef::Mat4(Mat4::IDENTITY),
        ),
        MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &keys[11],
            ExtensionValueRef::DenseArray(ExtensionArrayRef::Vec3(&empty_vec3)),
        ),
    ];
    let parts = build_scene_with_metadata(
        &account,
        &roots,
        &nodes,
        MetadataAuthoringBatch::new(&metadata),
    );

    inspect_single_node_profile(&parts.scene, |profile| {
        macro_rules! drain_lineage_domain {
            ($method:ident) => {
                loop {
                    if matches!(
                        profile.$method().expect(stringify!($method)),
                        NodeSceneProfileStep::End
                    ) {
                        break;
                    }
                }
            };
        }
        drain_lineage_domain!(step_resource);
        drain_lineage_domain!(step_source);
        drain_lineage_domain!(step_provenance);
        drain_lineage_domain!(step_evidence);
        let mut owner_count = 0_usize;
        while profile.next_owner().expect("metadata owner read").is_some() {
            owner_count += 1;
        }
        assert_eq!(owner_count, 3);

        for (index, key) in keys.iter().enumerate() {
            let entry = profile
                .next_metadata_entry()
                .expect("metadata entry read")
                .expect("metadata entry");
            assert_eq!(entry.owner(), CanonicalProfileOwner::Scene);
            assert_eq!(entry.key(), key.as_str());
            match (index, entry.value()) {
                (0, NodeSceneProfileMetadataValue::Bool(value)) => assert!(value),
                (1, NodeSceneProfileMetadataValue::I64(value)) => assert_eq!(value, -7),
                (2, NodeSceneProfileMetadataValue::U64(value)) => assert_eq!(value, 9),
                (3, NodeSceneProfileMetadataValue::F64(value)) => assert_eq!(value, 2.5),
                (4, NodeSceneProfileMetadataValue::String(value)) => {
                    assert_eq!(value, "metadata");
                }
                (
                    5,
                    NodeSceneProfileMetadataValue::Bytes {
                        media_type,
                        payload,
                    },
                ) => {
                    assert_eq!(media_type, "application/octet-stream");
                    assert_eq!(payload, &byte_payload);
                }
                (6, NodeSceneProfileMetadataValue::Vec2(values)) => {
                    assert_eq!(values, &[1.0, 2.0]);
                }
                (7, NodeSceneProfileMetadataValue::Vec3(values)) => {
                    assert_eq!(values, &[3.0, 4.0, 5.0]);
                }
                (8, NodeSceneProfileMetadataValue::Vec4(values)) => {
                    assert_eq!(values, &[6.0, 7.0, 8.0, 9.0]);
                }
                (9, NodeSceneProfileMetadataValue::Color(values)) => {
                    assert_eq!(values, &[0.1, 0.2, 0.3, 0.4]);
                }
                (10, NodeSceneProfileMetadataValue::Mat4(values)) => {
                    assert_eq!(
                        values,
                        &[
                            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
                            0.0, 1.0,
                        ]
                    );
                }
                (
                    11,
                    NodeSceneProfileMetadataValue::DenseArray {
                        element_kind,
                        logical_element_count,
                        values: NodeSceneProfileMetadataArray::F32(values),
                    },
                ) => {
                    assert_eq!(element_kind, NodeSceneProfileMetadataArrayKind::Vec3);
                    assert_eq!(logical_element_count, 0);
                    assert!(values.is_empty());
                }
                _ => panic!("metadata value kind or canonical ordering changed at {index}"),
            }
        }
        assert!(
            profile
                .next_metadata_entry()
                .expect("metadata entry end")
                .is_none()
        );
    });

    let (snapshot, transcript) =
        capture_node_scene_with_test_transcript(&parts.scene, snapshot_limits(16_384, 128), None)
            .expect("metadata snapshot");
    assert!(transcript.complete);
    assert_eq!(snapshot.totals.profile_metadata_entry_count, metadata.len());
    assert_eq!(snapshot.digest, KNOWN_GENERIC_METADATA_DIGEST);
    assert!(
        snapshot
            .preview
            .text
            .contains("profile_metadata(present,total=12)")
    );
    assert!(!snapshot.preview.bytes_truncated);
    assert!(!snapshot.preview.items_truncated);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn fresh_segment_nonce_is_excluded_from_the_semantic_snapshot() {
    let factory = factory();
    let account = account(&factory);
    let first = single_root_scene(&account, AuthoredTransform::identity_trs());
    let second = single_root_scene(&account, AuthoredTransform::identity_trs());
    let segment_nonce = |scene: &ValidatedNodeScene| {
        scene
            .canonical
            .records(scene.proof.profile_segments().into_store())
            .and_then(|segments| segments.scalar::<u64>(0, profile_segment_field::NONCE))
            .expect("validated fresh profile segment nonce")
    };
    assert_ne!(segment_nonce(&first.scene), segment_nonce(&second.scene));

    let (first_snapshot, first_transcript) =
        capture_node_scene_with_test_transcript(&first.scene, snapshot_limits(4_096, 64), None)
            .expect("first nonce-independent snapshot");
    let (second_snapshot, second_transcript) =
        capture_node_scene_with_test_transcript(&second.scene, snapshot_limits(4_096, 64), None)
            .expect("second nonce-independent snapshot");

    assert_eq!(first_snapshot.digest, second_snapshot.digest);
    assert_eq!(first_snapshot.preview.text, second_snapshot.preview.text);
    assert_eq!(first_snapshot.totals, second_snapshot.totals);
    assert_eq!(first_transcript.bytes, second_transcript.bytes);

    drop(second);
    drop(first);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn snapshot_semantics_are_independent_of_validated_profile_segment_partitioning() {
    const SOURCE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(0)];
    const OPERATION_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(1)];

    let factory = factory();
    let account = account(&factory);
    let resource_identity = ResourceIdentity::new("fixtures/snapshot/segment-partitioning.glb")
        .expect("segment-partitioning resource identity");
    let format = FormatId::new("org.baozi.snapshot-segment-partitioning")
        .expect("segment-partitioning format identity");
    let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
    let resources = [ObservedResourceAuthoring::session_opaque(
        &resource_identity,
        SessionOpaqueReason::CallerLabel,
    )];
    let sources = [SourceIdentityAuthoring::new(
        ResourceIdentityLocalId::new(0),
        &format,
        &locator,
    )];
    let operation_kind = OperationKind::Transform;
    let options_digest = OptionsDigest::sha256([0x5a; 32]);
    let operation =
        ProvenanceOperationAuthoring::new(&operation_kind, options_digest, &SOURCE_PROVENANCE, &[]);
    let source_only = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
        0,
    ))];
    let source_then_operation = [
        ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(0)),
        ProvenanceNodeAuthoring::operation(operation),
    ];
    let source_owners = [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &SOURCE_PROVENANCE),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &SOURCE_PROVENANCE),
        OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(0)), &SOURCE_PROVENANCE),
    ];
    let final_owners = [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &SOURCE_PROVENANCE),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &SOURCE_PROVENANCE),
        OwnerProvenanceAuthoring::new(
            ProfileOwner::Node(NodeLocalId::new(0)),
            &OPERATION_PROVENANCE,
        ),
    ];
    let roots = [NodeLocalId::new(0)];
    let initial_nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    let final_transform = trs(23.0);
    let final_nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        final_transform,
        Presence::Absent,
        Presence::Absent,
    )];

    let base_input = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &initial_nodes),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::new(&[], &[], &[], &[], &[]),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&resources, &sources, &source_only, &source_owners),
            MetadataAuthoringBatch::empty(),
        ),
    );
    let base = publish_scene_input(&account, base_input);
    let node = NodeId::new(base.scene.generation(), 0);
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(&base.scene, node, final_transform, options_digest)
            .expect("segment-partitioning transform edit must start")
    else {
        panic!("different authored transform must require a SceneEdit");
    };
    let prepared = pending
        .prepare(operation_limits(), OperationControl::default())
        .expect("segment-partitioning transform edit must prepare");
    let NodeTransformEditTerminal::Success(edited) = prepared.handoff() else {
        panic!("segment-partitioning transform edit must publish");
    };

    let direct_input = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &final_nodes),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::new(&[], &[], &[], &[], &[]),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&resources, &sources, &source_then_operation, &final_owners),
            MetadataAuthoringBatch::empty(),
        ),
    );
    let direct = publish_scene_input(&account, direct_input);

    assert_eq!(direct.scene.proof.profile_segment_count(), 1);
    assert_eq!(edited.scene.proof.profile_segment_count(), 2);
    let limits = snapshot_limits(MAX_PREVIEW_BYTES, MAX_COLLECTION_ITEMS);
    let (direct_snapshot, direct_transcript) =
        capture_node_scene_with_test_transcript(&direct.scene, limits, None)
            .expect("single-segment semantic snapshot");
    let (edited_snapshot, edited_transcript) =
        capture_node_scene_with_test_transcript(&edited.scene, limits, None)
            .expect("two-segment semantic snapshot");

    assert!(direct_transcript.complete);
    assert!(edited_transcript.complete);
    assert_eq!(direct_snapshot.domain, edited_snapshot.domain);
    assert_eq!(
        direct_snapshot.schema_revision,
        edited_snapshot.schema_revision
    );
    assert_eq!(direct_snapshot.algorithm, edited_snapshot.algorithm);
    assert_eq!(direct_snapshot.digest, edited_snapshot.digest);
    assert_eq!(direct_snapshot.preview, edited_snapshot.preview);
    assert_eq!(direct_transcript.bytes, edited_transcript.bytes);
    let direct_work = direct_snapshot.totals.work_units;
    let edited_work = edited_snapshot.totals.work_units;
    let mut direct_totals = direct_snapshot.totals;
    let mut edited_totals = edited_snapshot.totals;
    direct_totals.work_units = 0;
    edited_totals.work_units = 0;
    assert_eq!(direct_totals, edited_totals);
    assert!(edited_work > direct_work);

    drop(direct);
    drop(edited);
    drop(base);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn profile_cursor_requires_each_global_lineage_phase_to_be_exhausted() {
    let factory = factory();
    let account = account(&factory);
    let parts = single_root_scene(&account, AuthoredTransform::identity_trs());
    let view = parts.scene.semantic_view().expect("profile semantic view");
    let mut cursor = view.into_cursor().expect("profile semantic cursor");

    let mut roots = cursor.take_roots().expect("profile roots");
    assert_eq!(roots.next().expect("profile root"), Some(0));
    assert_eq!(roots.next().expect("profile root end"), None);
    roots.finish().expect("complete profile roots");

    let node = cursor
        .next_node()
        .expect("profile node read")
        .expect("profile node");
    node.into_children()
        .finish()
        .expect("complete profile node children");
    assert!(cursor.next_node().expect("profile node end").is_none());

    {
        let profile = cursor.profile().expect("profile cursor");
        assert!(profile.step_source().is_err());
        assert!(matches!(
            profile
                .step_resource()
                .expect("profile resource checkpoint"),
            NodeSceneProfileStep::Checkpoint
        ));
        assert!(matches!(
            profile.step_resource().expect("profile resource read"),
            NodeSceneProfileStep::Item(_)
        ));
        assert!(profile.step_source().is_err());
        assert!(matches!(
            profile.step_resource().expect("profile resource end"),
            NodeSceneProfileStep::End
        ));
        assert!(matches!(
            profile.step_source().expect("profile source checkpoint"),
            NodeSceneProfileStep::Checkpoint
        ));
        assert!(matches!(
            profile.step_source().expect("profile source read"),
            NodeSceneProfileStep::Item(_)
        ));
        assert!(profile.next_owner().is_err());
    }

    assert!(cursor.finish().is_err());
}

#[test]
fn profile_steps_bound_empty_segments_without_replaying_prior_domains() {
    const EDIT_COUNT: usize = 32;

    let factory = factory();
    let account = account(&factory);
    let NodeSceneSuccessParts { mut scene, report } =
        single_root_scene(&account, AuthoredTransform::identity_trs());
    drop(report);

    for ordinal in 0..EDIT_COUNT {
        let node = NodeId::new(scene.generation(), 0);
        let NodeTransformEditStart::Changed(pending) = try_begin_node_transform_edit(
            &scene,
            node,
            trs(ordinal as f32 + 1.0),
            OptionsDigest::sha256([u8::try_from(ordinal).expect("bounded edit ordinal"); 32]),
        )
        .expect("bounded profile edit start") else {
            panic!("each transform edit must append one profile segment");
        };
        let prepared = pending
            .prepare(operation_limits(), OperationControl::default())
            .expect("bounded profile edit preparation");
        let NodeTransformEditTerminal::Success(edited) = prepared.handoff() else {
            panic!("bounded profile edit must publish");
        };
        let previous = scene;
        scene = edited.scene;
        drop(edited.report);
        drop(previous);
    }

    let full = capture_node_scene(
        &scene,
        snapshot_limits(MAX_PREVIEW_BYTES, MAX_COLLECTION_ITEMS),
        None,
    )
    .expect("multi-segment profile snapshot");
    let exact = NodeSceneSnapshotLimits::try_new(
        MAX_PREVIEW_BYTES,
        MAX_COLLECTION_ITEMS,
        full.totals.work_units,
    )
    .expect("exact multi-segment snapshot work");
    capture_node_scene(&scene, exact, None).expect("exact profile checkpoint budget");
    let one_short = NodeSceneSnapshotLimits::try_new(
        MAX_PREVIEW_BYTES,
        MAX_COLLECTION_ITEMS,
        full.totals
            .work_units
            .checked_sub(1)
            .expect("nonzero multi-segment snapshot work"),
    )
    .expect("one-short multi-segment snapshot work");
    assert!(matches!(
        capture_node_scene(&scene, one_short, None),
        Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. })
    ));

    let view = scene.semantic_view().expect("multi-segment semantic view");
    let mut cursor = view.into_cursor().expect("multi-segment semantic cursor");
    let mut roots = cursor.take_roots().expect("multi-segment roots");
    assert_eq!(roots.next().expect("multi-segment root"), Some(0));
    assert_eq!(roots.next().expect("multi-segment root end"), None);
    roots.finish().expect("complete multi-segment roots");
    let node = cursor
        .next_node()
        .expect("multi-segment node read")
        .expect("multi-segment node");
    node.into_children()
        .finish()
        .expect("complete multi-segment node children");
    assert!(
        cursor
            .next_node()
            .expect("multi-segment node end")
            .is_none()
    );

    {
        let appearance = cursor.appearance().expect("multi-segment appearance");
        assert!(appearance.next_image().expect("image end").is_none());
        assert!(appearance.next_sampler().expect("sampler end").is_none());
        assert!(appearance.next_texture().expect("texture end").is_none());
        assert!(appearance.next_material().expect("material end").is_none());
        assert!(appearance.next_binding().expect("binding end").is_none());
    }
    {
        let geometry = cursor.geometry().expect("multi-segment geometry");
        assert!(
            geometry
                .next_logical_mesh()
                .expect("logical mesh end")
                .is_none()
        );
        assert!(geometry.next_primitive().expect("primitive end").is_none());
        assert!(geometry.next_attribute().expect("attribute end").is_none());
        assert!(
            geometry
                .next_mesh_instance()
                .expect("mesh instance end")
                .is_none()
        );
    }
    {
        let deformation = cursor.deformation().expect("multi-segment deformation");
        assert!(deformation.next_skin().expect("skin end").is_none());
        assert!(
            deformation
                .next_logical_morph()
                .expect("logical morph end")
                .is_none()
        );
        assert!(
            deformation
                .next_morph_definition()
                .expect("morph definition end")
                .is_none()
        );
        assert!(
            deformation
                .next_primitive()
                .expect("deformation primitive end")
                .is_none()
        );
        assert!(
            deformation
                .next_morph_target()
                .expect("morph target end")
                .is_none()
        );
        assert!(
            deformation
                .next_morph_delta()
                .expect("morph delta end")
                .is_none()
        );
        assert!(
            deformation
                .next_mesh_instance()
                .expect("deformation mesh instance end")
                .is_none()
        );
    }
    {
        let animation = cursor.animation().expect("multi-segment animation");
        assert!(animation.next_clip().expect("animation clip end").is_none());
        assert!(
            animation
                .next_channel()
                .expect("animation channel end")
                .is_none()
        );
    }

    let profile = cursor.profile().expect("multi-segment profile cursor");
    macro_rules! drain_lineage_domain {
        ($method:ident) => {{
            let mut checkpoints = 0_usize;
            let mut items = 0_usize;
            loop {
                match profile.$method().expect(stringify!($method)) {
                    NodeSceneProfileStep::Checkpoint => checkpoints += 1,
                    NodeSceneProfileStep::Item(_) => items += 1,
                    NodeSceneProfileStep::End => break,
                }
            }
            (checkpoints, items)
        }};
    }
    let segment_count = EDIT_COUNT + 1;
    assert_eq!(drain_lineage_domain!(step_resource), (segment_count, 1));
    assert_eq!(drain_lineage_domain!(step_source), (segment_count, 1));
    assert_eq!(
        drain_lineage_domain!(step_provenance),
        (segment_count, segment_count)
    );
    assert_eq!(drain_lineage_domain!(step_evidence), (segment_count, 0));
    assert_eq!(
        profile.decoded_lineage_rows_for_test(),
        [1, 1, segment_count, 0]
    );
    assert_eq!(
        profile.lineage_segment_checkpoints_for_test(),
        [segment_count; 4]
    );
    while profile.next_owner().expect("profile owner read").is_some() {}
    while profile
        .next_metadata_entry()
        .expect("profile metadata read")
        .is_some()
    {}
    cursor.finish().expect("complete multi-segment traversal");

    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn changed_scene_edit_appends_snapshot_visible_profile_lineage() {
    let factory = factory();
    let account = account(&factory);
    let source = single_root_scene(&account, AuthoredTransform::identity_trs());
    let source_snapshot = capture_node_scene(
        &source.scene,
        snapshot_limits(MAX_PREVIEW_BYTES, MAX_COLLECTION_ITEMS),
        None,
    )
    .expect("source profile snapshot");
    let node = NodeId::new(source.scene.generation(), 0);
    let NodeTransformEditStart::Changed(pending) = try_begin_node_transform_edit(
        &source.scene,
        node,
        trs(9.0),
        OptionsDigest::sha256([0x5a; 32]),
    )
    .expect("changed transform profile edit") else {
        panic!("changed transform must append profile lineage");
    };
    let prepared = pending
        .prepare(operation_limits(), OperationControl::default())
        .expect("changed transform profile preparation");
    let NodeTransformEditTerminal::Success(edited) = prepared.handoff() else {
        panic!("valid changed transform must publish an edited Scene");
    };

    let snapshot = capture_node_scene(
        &edited.scene,
        snapshot_limits(MAX_PREVIEW_BYTES, MAX_COLLECTION_ITEMS),
        None,
    )
    .expect("edited profile snapshot");
    assert_ne!(snapshot.digest, source_snapshot.digest);
    assert_eq!(snapshot.totals.profile_resource_count, 1);
    assert_eq!(snapshot.totals.profile_source_count, 1);
    assert_eq!(snapshot.totals.profile_provenance_node_count, 2);
    assert_eq!(snapshot.totals.profile_operation_count, 1);
    assert_eq!(snapshot.totals.profile_evidence_count, 0);
    assert_eq!(snapshot.totals.profile_parent_reference_count, 1);
    assert_eq!(snapshot.totals.profile_owner_count, 3);
    assert_eq!(snapshot.totals.profile_attachment_count, 3);
    assert_eq!(snapshot.totals.profile_metadata_entry_count, 0);
    assert_eq!(snapshot.totals.profile_lineage_digest_bytes, 32);
    assert!(snapshot.preview.text.contains("profile_provenance[1]"));

    drop(edited);
    drop(source);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn spatial_known_vector_pins_scene_space_cameras_lights_and_attachments() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Present(CameraLocalId::new(0)),
        Presence::Present(LightLocalId::new(0)),
    )];
    let scene_space = namespaced_scene_space();
    let projection = CameraProjection::perspective(1.0, Some(1.5), 0.1, Some(100.0))
        .expect("spatial known-vector camera");
    let cameras = [CameraAuthoring::new(projection)];
    let light_kind =
        LightKind::spot(Some(50.0), 0.25, 0.75).expect("spatial known-vector spot light");
    let emission = white_emission(10.0);
    let lights = [LightAuthoring::new(light_kind, emission)];
    let parts = build_scene_with_spatial_appearance_and_geometry(
        &account,
        &roots,
        &nodes,
        SpatialAuthoringBatch::new(&scene_space, &cameras, &lights),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
    );

    let view = parts.scene.semantic_view().expect("spatial semantic view");
    assert_eq!(
        view.scene_space().expect("SceneSpace view").up_axis(),
        Axis::NegativeY
    );
    assert_eq!(
        view.camera_at(0)
            .expect("camera view")
            .expect("camera zero")
            .projection(),
        projection
    );
    assert_eq!(
        view.light_at(0)
            .expect("light view")
            .expect("light zero")
            .kind(),
        light_kind
    );
    let mut spatial_placements = view.spatial_placements();
    let placement = spatial_placements
        .next()
        .expect("spatial placement")
        .expect("source-node placement");
    assert_eq!(placement.source_node(), 0);
    assert_eq!(placement.camera(), Some(0));
    assert_eq!(placement.light(), Some(0));
    assert_eq!(
        spatial_placements.next().expect("spatial placement end"),
        None
    );
    spatial_placements
        .finish()
        .expect("complete spatial placement scan");
    let mut camera_placements = view.camera_placements(0).expect("camera placements");
    assert_eq!(camera_placements.next().expect("camera placement"), Some(0));
    assert_eq!(
        camera_placements.next().expect("camera placement end"),
        None
    );
    camera_placements
        .finish()
        .expect("complete camera placement scan");
    let mut light_placements = view.light_placements(0).expect("light placements");
    assert_eq!(light_placements.next().expect("light placement"), Some(0));
    assert_eq!(light_placements.next().expect("light placement end"), None);
    light_placements
        .finish()
        .expect("complete light placement scan");
    let mut cursor = view.into_cursor().expect("spatial node cursor");
    let mut root_cursor = cursor.take_roots().expect("spatial roots");
    assert_eq!(root_cursor.next().expect("spatial root"), Some(0));
    assert_eq!(root_cursor.next().expect("spatial root end"), None);
    root_cursor.finish().expect("complete spatial root scan");
    let node = cursor
        .next_node()
        .expect("spatial node")
        .expect("source node zero");
    assert_eq!(node.camera(), Some(0));
    assert_eq!(node.light(), Some(0));

    let (snapshot, transcript) =
        capture_node_scene_with_test_transcript(&parts.scene, snapshot_limits(16_384, 64), None)
            .expect("spatial known-vector snapshot");
    assert_eq!(snapshot.digest, KNOWN_SPATIAL_DIGEST);
    assert!(transcript.complete);
    assert_hex_parts(
        &transcript.bytes,
        &[
            KNOWN_SPATIAL_TRANSCRIPT_PREFIX_HEX,
            EMPTY_PRE_PROFILE_DOMAINS_TRANSCRIPT_HEX,
            FRESH_PROFILE_TRANSCRIPT_PREFIX_HEX,
            PROFILE_OWNER_COLLECTION_5_HEX,
            PROFILE_SCENE_OWNER_ROW_HEX,
            PROFILE_SYNTHETIC_ROOT_OWNER_ROW_HEX,
            PROFILE_NODE_0_OWNER_ROW_HEX,
            PROFILE_CAMERA_0_OWNER_ROW_HEX,
            PROFILE_LIGHT_0_OWNER_ROW_HEX,
            EMPTY_PROFILE_METADATA_TRANSCRIPT_HEX,
        ],
    );
    assert_eq!(snapshot.totals.transcript_bytes, 1_309);
    assert_eq!(snapshot.totals.work_units, 134);
    assert_eq!(snapshot.totals.camera_count, 1);
    assert_eq!(snapshot.totals.light_count, 1);
    assert_eq!(snapshot.totals.camera_attachment_count, 1);
    assert_eq!(snapshot.totals.light_attachment_count, 1);
    assert!(snapshot.preview.text.contains(concat!(
        "scene_space handedness=left up_axis=negative_y front_axis=positive_z ",
        "unit_scale_to_meters_bits=0x3f847ae147ae147b ",
        "front_face_winding=clockwise uv_origin=lower_left ",
        "transform_convention=column_vector_local_to_parent ",
        "tangent_basis=namespaced tangent_basis_schema_name=example:tangent-basis ",
        "tangent_basis_schema_version=2 tangent_handedness_encoding=namespaced ",
        "tangent_handedness_encoding_schema_name=example:tangent-sign ",
        "tangent_handedness_encoding_schema_version=3\n",
    )));
    assert!(snapshot.preview.text.contains(concat!(
        "camera[0] projection=perspective vertical_fov=0x3f800000 ",
        "aspect_ratio=0x3fc00000 near_plane=0x3dcccccd far_plane=0x42c80000",
    )));
    assert!(snapshot.preview.text.contains(concat!(
        "light[0] kind=spot range=0x42480000 inner_cone=0x3e800000 ",
        "outer_cone=0x3f400000 tint_r=0x3f800000 tint_g=0x3f800000 ",
        "tint_b=0x3f800000 intensity=0x41200000",
    )));
    assert!(snapshot.preview.text.contains("camera=0 light=0"));
    assert!(!snapshot.preview.bytes_truncated);
    assert!(!snapshot.preview.items_truncated);
}

#[test]
fn scene_space_field_mutation_vectors_are_digest_visible() {
    let factory = factory();
    let account = account(&factory);
    let baseline = namespaced_scene_space();
    let projection = CameraProjection::perspective(1.0, Some(1.5), 0.1, Some(100.0))
        .expect("SceneSpace mutation camera");
    let light_kind = LightKind::spot(Some(50.0), 0.25, 0.75).expect("SceneSpace mutation light");
    let emission = white_emission(10.0);
    let digest = |scene_space: &SceneSpace| {
        capture_single_spatial_digest(&account, scene_space, projection, light_kind, emission)
    };
    let baseline_digest = digest(&baseline);
    let mutations = [
        (
            "SceneSpace handedness",
            SceneSpace {
                handedness: Handedness::Right,
                ..baseline.clone()
            },
        ),
        (
            "SceneSpace up axis",
            SceneSpace {
                up_axis: Axis::PositiveX,
                ..baseline.clone()
            },
        ),
        (
            "SceneSpace front axis",
            SceneSpace {
                front_axis: Axis::NegativeX,
                ..baseline.clone()
            },
        ),
        (
            "SceneSpace unit-scale bits",
            SceneSpace {
                unit_scale_to_meters: Some(0.02),
                ..baseline.clone()
            },
        ),
        (
            "SceneSpace unit-scale presence",
            SceneSpace {
                unit_scale_to_meters: None,
                ..baseline.clone()
            },
        ),
        (
            "SceneSpace front-face winding",
            SceneSpace {
                front_face_winding: FrontFaceWinding::CounterClockwise,
                ..baseline.clone()
            },
        ),
        (
            "SceneSpace UV origin",
            SceneSpace {
                uv_origin: UvOrigin::UpperLeft,
                ..baseline.clone()
            },
        ),
        (
            "SceneSpace transform convention",
            SceneSpace {
                transform_convention: TransformConvention::Unknown,
                ..baseline.clone()
            },
        ),
        (
            "tangent-basis schema name",
            SceneSpace {
                tangent_basis: TangentBasisConvention::try_namespaced(Some(extension_schema(
                    "example:tangent-basis-v2",
                    2,
                )))
                .expect("mutated tangent-basis name"),
                ..baseline.clone()
            },
        ),
        (
            "tangent-basis schema version",
            SceneSpace {
                tangent_basis: TangentBasisConvention::try_namespaced(Some(extension_schema(
                    "example:tangent-basis",
                    4,
                )))
                .expect("mutated tangent-basis version"),
                ..baseline.clone()
            },
        ),
        (
            "tangent-encoding schema name",
            SceneSpace {
                tangent_handedness_encoding: TangentHandednessEncoding::try_namespaced(Some(
                    extension_schema("example:tangent-sign-v2", 3),
                ))
                .expect("mutated tangent-encoding name"),
                ..baseline.clone()
            },
        ),
        (
            "tangent-encoding schema version",
            SceneSpace {
                tangent_handedness_encoding: TangentHandednessEncoding::try_namespaced(Some(
                    extension_schema("example:tangent-sign", 5),
                ))
                .expect("mutated tangent-encoding version"),
                ..baseline.clone()
            },
        ),
    ];

    for (label, mutation) in mutations {
        assert_digest_mutation(label, baseline_digest, digest(&mutation));
    }

    let basis_category_baseline = SceneSpace {
        tangent_handedness_encoding: TangentHandednessEncoding::SignInW,
        ..baseline.clone()
    };
    let basis_category_mutation = SceneSpace {
        tangent_basis: TangentBasisConvention::MikkTspace,
        ..basis_category_baseline.clone()
    };
    assert_digest_mutation(
        "tangent-basis category",
        digest(&basis_category_baseline),
        digest(&basis_category_mutation),
    );
    let encoding_category_mutation = SceneSpace {
        tangent_handedness_encoding: TangentHandednessEncoding::SignInW,
        ..baseline.clone()
    };
    assert_digest_mutation(
        "tangent-handedness encoding category",
        baseline_digest,
        digest(&encoding_category_mutation),
    );
}

#[test]
fn camera_and_light_field_mutation_vectors_are_digest_visible() {
    let factory = factory();
    let account = account(&factory);
    let scene_space = namespaced_scene_space();
    let baseline_projection = CameraProjection::perspective(1.0, Some(1.5), 0.1, Some(100.0))
        .expect("camera mutation baseline");
    let baseline_kind = LightKind::spot(Some(50.0), 0.25, 0.75).expect("light mutation baseline");
    let baseline_emission = white_emission(10.0);
    let camera_digest = |projection| {
        capture_single_spatial_digest(
            &account,
            &scene_space,
            projection,
            baseline_kind,
            baseline_emission,
        )
    };
    let camera_baseline_digest = camera_digest(baseline_projection);
    let camera_mutations = [
        (
            "camera projection kind",
            CameraProjection::orthographic(1.0, 1.5, 0.1, 100.0)
                .expect("orthographic projection-kind mutation"),
        ),
        (
            "camera vertical field of view",
            CameraProjection::perspective(1.1, Some(1.5), 0.1, Some(100.0))
                .expect("vertical-field-of-view mutation"),
        ),
        (
            "camera aspect-ratio bits",
            CameraProjection::perspective(1.0, Some(1.6), 0.1, Some(100.0))
                .expect("aspect-ratio bits mutation"),
        ),
        (
            "camera aspect-ratio presence",
            CameraProjection::perspective(1.0, None, 0.1, Some(100.0))
                .expect("aspect-ratio presence mutation"),
        ),
        (
            "camera near plane",
            CameraProjection::perspective(1.0, Some(1.5), 0.2, Some(100.0))
                .expect("near-plane mutation"),
        ),
        (
            "camera far-plane bits",
            CameraProjection::perspective(1.0, Some(1.5), 0.1, Some(101.0))
                .expect("far-plane bits mutation"),
        ),
        (
            "camera far-plane presence",
            CameraProjection::perspective(1.0, Some(1.5), 0.1, None)
                .expect("far-plane presence mutation"),
        ),
    ];
    for (label, mutation) in camera_mutations {
        assert_digest_mutation(label, camera_baseline_digest, camera_digest(mutation));
    }

    let orthographic =
        CameraProjection::orthographic(2.0, 3.0, 0.0, 100.0).expect("orthographic preview camera");
    let orthographic_baseline_digest = camera_digest(orthographic);
    for (label, mutation) in [
        (
            "camera horizontal half extent",
            CameraProjection::orthographic(2.5, 3.0, 0.0, 100.0)
                .expect("horizontal-half-extent mutation"),
        ),
        (
            "camera vertical half extent",
            CameraProjection::orthographic(2.0, 3.5, 0.0, 100.0)
                .expect("vertical-half-extent mutation"),
        ),
        (
            "orthographic camera near plane",
            CameraProjection::orthographic(2.0, 3.0, 0.1, 100.0)
                .expect("orthographic near-plane mutation"),
        ),
        (
            "orthographic camera far plane",
            CameraProjection::orthographic(2.0, 3.0, 0.0, 101.0)
                .expect("orthographic far-plane mutation"),
        ),
    ] {
        assert_digest_mutation(label, orthographic_baseline_digest, camera_digest(mutation));
    }
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Present(CameraLocalId::new(0)),
        Presence::Present(LightLocalId::new(0)),
    )];
    let cameras = [CameraAuthoring::new(orthographic)];
    let lights = [LightAuthoring::new(baseline_kind, baseline_emission)];
    let orthographic_snapshot =
        capture_spatial_scene(&account, &roots, &nodes, &scene_space, &cameras, &lights);
    assert!(orthographic_snapshot.preview.text.contains(concat!(
        "camera[0] projection=orthographic horizontal_half_extent=0x40000000 ",
        "vertical_half_extent=0x40400000 near_plane=0x00000000 ",
        "far_plane=0x42c80000",
    )));

    let light_digest = |kind, emission| {
        capture_single_spatial_digest(&account, &scene_space, baseline_projection, kind, emission)
    };
    let light_baseline_digest = light_digest(baseline_kind, baseline_emission);
    for (label, mutation) in [
        (
            "light range bits",
            LightKind::spot(Some(51.0), 0.25, 0.75).expect("light range-bits mutation"),
        ),
        (
            "light range presence",
            LightKind::spot(None, 0.25, 0.75).expect("light range-presence mutation"),
        ),
        (
            "light inner cone",
            LightKind::spot(Some(50.0), 0.3, 0.75).expect("light inner-cone mutation"),
        ),
        (
            "light outer cone",
            LightKind::spot(Some(50.0), 0.25, 0.8).expect("light outer-cone mutation"),
        ),
    ] {
        assert_digest_mutation(
            label,
            light_baseline_digest,
            light_digest(mutation, baseline_emission),
        );
    }
    assert_digest_mutation(
        "light kind category",
        light_digest(LightKind::directional(), baseline_emission),
        light_digest(
            LightKind::point(None).expect("point category mutation"),
            baseline_emission,
        ),
    );

    let tinted_emission = LightEmission::try_from_scaled_linear_srgb(
        Color::linear_rgba(2.0, 1.0, 0.5, 1.0),
        f32::from_bits(0x4107_ff21),
    )
    .expect("isolated light-tint mutation");
    assert_eq!(
        tinted_emission.intensity().to_bits(),
        baseline_emission.intensity().to_bits(),
        "tint vector must hold canonical intensity constant",
    );
    assert_ne!(tinted_emission.tint(), baseline_emission.tint());
    assert_digest_mutation(
        "light tint",
        light_baseline_digest,
        light_digest(baseline_kind, tinted_emission),
    );

    let stronger_emission = white_emission(11.0);
    assert_eq!(stronger_emission.tint(), baseline_emission.tint());
    assert_digest_mutation(
        "light intensity",
        light_baseline_digest,
        light_digest(baseline_kind, stronger_emission),
    );
}

#[test]
fn spatial_attachment_presence_and_reference_mutations_are_digest_visible() {
    let factory = factory();
    let account = account(&factory);
    let scene_space = namespaced_scene_space();
    let projection = CameraProjection::perspective(1.0, Some(1.5), 0.1, Some(100.0))
        .expect("attachment mutation camera");
    let light_kind = LightKind::spot(Some(50.0), 0.25, 0.75).expect("attachment mutation light");
    let emission = white_emission(10.0);
    let roots = [NodeLocalId::new(0), NodeLocalId::new(1)];
    let cameras = [CameraAuthoring::new(projection)];
    let lights = [LightAuthoring::new(light_kind, emission)];
    let both_attached = [
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(0)),
            Presence::Present(LightLocalId::new(0)),
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(0)),
            Presence::Present(LightLocalId::new(0)),
        ),
    ];
    let presence_baseline = capture_spatial_scene(
        &account,
        &roots,
        &both_attached,
        &scene_space,
        &cameras,
        &lights,
    )
    .digest;
    let camera_absent = [
        both_attached[0],
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Present(LightLocalId::new(0)),
        ),
    ];
    assert_digest_mutation(
        "camera attachment presence",
        presence_baseline,
        capture_spatial_scene(
            &account,
            &roots,
            &camera_absent,
            &scene_space,
            &cameras,
            &lights,
        )
        .digest,
    );
    let light_absent = [
        both_attached[0],
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(0)),
            Presence::Absent,
        ),
    ];
    assert_digest_mutation(
        "light attachment presence",
        presence_baseline,
        capture_spatial_scene(
            &account,
            &roots,
            &light_absent,
            &scene_space,
            &cameras,
            &lights,
        )
        .digest,
    );

    let two_cameras = [
        CameraAuthoring::new(projection),
        CameraAuthoring::new(projection),
    ];
    let two_lights = [
        LightAuthoring::new(light_kind, emission),
        LightAuthoring::new(light_kind, emission),
    ];
    let reference_baseline_nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(0)),
            Presence::Present(LightLocalId::new(0)),
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(1)),
            Presence::Present(LightLocalId::new(1)),
        ),
    ];
    let reference_baseline = capture_spatial_scene(
        &account,
        &roots,
        &reference_baseline_nodes,
        &scene_space,
        &two_cameras,
        &two_lights,
    )
    .digest;
    let swapped_camera_references = [
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(1)),
            Presence::Present(LightLocalId::new(0)),
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(0)),
            Presence::Present(LightLocalId::new(1)),
        ),
    ];
    assert_digest_mutation(
        "camera attachment reference",
        reference_baseline,
        capture_spatial_scene(
            &account,
            &roots,
            &swapped_camera_references,
            &scene_space,
            &two_cameras,
            &two_lights,
        )
        .digest,
    );
    let swapped_light_references = [
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(0)),
            Presence::Present(LightLocalId::new(1)),
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(1)),
            Presence::Present(LightLocalId::new(0)),
        ),
    ];
    assert_digest_mutation(
        "light attachment reference",
        reference_baseline,
        capture_spatial_scene(
            &account,
            &roots,
            &swapped_light_references,
            &scene_space,
            &two_cameras,
            &two_lights,
        )
        .digest,
    );
}

#[test]
fn matrix_and_child_known_vector_pins_node_framing_and_empty_domains() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0)];
    let nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            matrix(4.0),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(parent(0, 0), trs(8.0), Presence::Absent, Presence::Absent),
    ];
    let parts = build_scene(&account, &roots, &nodes);

    let (snapshot, transcript) =
        capture_node_scene_with_test_transcript(&parts.scene, snapshot_limits(8_192, 64), None)
            .expect("matrix-child known Scene snapshot");

    assert_eq!(snapshot.digest, KNOWN_MATRIX_CHILD_DIGEST);
    assert!(transcript.complete);
    assert_hex_parts(
        &transcript.bytes,
        &[
            KNOWN_MATRIX_CHILD_TRANSCRIPT_PREFIX_HEX,
            EMPTY_PRE_PROFILE_DOMAINS_TRANSCRIPT_HEX,
            FRESH_PROFILE_TRANSCRIPT_PREFIX_HEX,
            PROFILE_OWNER_COLLECTION_4_HEX,
            PROFILE_SCENE_OWNER_ROW_HEX,
            PROFILE_SYNTHETIC_ROOT_OWNER_ROW_HEX,
            PROFILE_NODE_0_OWNER_ROW_HEX,
            PROFILE_NODE_1_OWNER_ROW_HEX,
            EMPTY_PROFILE_METADATA_TRANSCRIPT_HEX,
        ],
    );
    assert_eq!(snapshot.totals.node_count, 2);
    assert_eq!(snapshot.totals.root_count, 1);
    assert_eq!(snapshot.totals.edge_count, 1);
    assert_eq!(snapshot.totals.transcript_bytes, 1_190);
    assert_eq!(snapshot.totals.work_units, 137);
}

#[test]
fn point_geometry_known_vector_pins_complete_scene_framing() {
    let factory = factory();
    let account = account(&factory);
    let positions = [1.0_f32, 2.0, 3.0];
    let parts = point_geometry_scene(&account, &positions);

    let (snapshot, transcript) =
        capture_node_scene_with_test_transcript(&parts.scene, snapshot_limits(8_192, 64), None)
            .expect("point-geometry known Scene snapshot");

    assert_eq!(snapshot.digest, KNOWN_POINT_GEOMETRY_DIGEST);
    assert!(transcript.complete);
    assert_hex_parts(
        &transcript.bytes,
        &[
            KNOWN_POINT_GEOMETRY_TRANSCRIPT_HEX,
            FRESH_PROFILE_TRANSCRIPT_PREFIX_HEX,
            PROFILE_OWNER_COLLECTION_8_HEX,
            PROFILE_SCENE_OWNER_ROW_HEX,
            PROFILE_SYNTHETIC_ROOT_OWNER_ROW_HEX,
            PROFILE_NODE_0_OWNER_ROW_HEX,
            PROFILE_MATERIAL_0_OWNER_ROW_HEX,
            PROFILE_LOGICAL_MESH_0_OWNER_ROW_HEX,
            PROFILE_PRIMITIVE_0_OWNER_ROW_HEX,
            PROFILE_ATTRIBUTE_0_OWNER_ROW_HEX,
            PROFILE_MESH_INSTANCE_0_OWNER_ROW_HEX,
            EMPTY_PROFILE_METADATA_TRANSCRIPT_HEX,
        ],
    );
    assert_eq!(snapshot.totals.node_count, 1);
    assert_eq!(snapshot.totals.material_count, 1);
    assert_eq!(snapshot.totals.logical_mesh_count, 1);
    assert_eq!(snapshot.totals.primitive_count, 1);
    assert_eq!(snapshot.totals.attribute_count, 1);
    assert_eq!(snapshot.totals.mesh_instance_count, 1);
    assert_eq!(snapshot.totals.logical_morph_count, 1);
    assert_eq!(snapshot.totals.primitive_deformation_count, 1);
    assert_eq!(snapshot.totals.deformation_mesh_instance_count, 1);
    assert_eq!(snapshot.totals.skin_count, 0);
    assert_eq!(snapshot.totals.morph_definition_count, 0);
    assert_eq!(snapshot.totals.morph_target_count, 0);
    assert_eq!(snapshot.totals.morph_delta_count, 0);
    assert_eq!(snapshot.totals.polygon_boundary_count, 0);
    assert_eq!(snapshot.totals.primitive_index_count, 0);
    assert_eq!(snapshot.totals.sparse_index_count, 0);
    assert_eq!(snapshot.totals.canonical_scalar_count, 3);
    assert_eq!(snapshot.totals.f32_scalar_count, 3);
    assert_eq!(snapshot.totals.u32_scalar_count, 0);
    assert_eq!(snapshot.totals.transcript_bytes, 1_871);
    assert_eq!(snapshot.totals.work_units, 194);
    assert!(snapshot.preview.text.contains("material[0]"));
    assert!(
        snapshot
            .preview
            .text
            .contains("primitive[0] topology=points")
    );
    assert!(snapshot.preview.text.contains("values(present,total=3)"));
    assert!(
        snapshot
            .preview
            .text
            .contains("mesh_instance[0] owner_node=0")
    );
}

#[test]
fn collection_framing_distinguishes_root_child_partitions() {
    let factory = factory();
    let account = account(&factory);
    let first_roots = [NodeLocalId::new(0)];
    let first_nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            trs(0.0),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(parent(0, 0), trs(1.0), Presence::Absent, Presence::Absent),
        NodeAuthoringNode::new(parent(0, 1), trs(2.0), Presence::Absent, Presence::Absent),
        NodeAuthoringNode::new(parent(1, 0), trs(3.0), Presence::Absent, Presence::Absent),
    ];
    let second_roots = [NodeLocalId::new(0)];
    let second_nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            trs(0.0),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(parent(0, 0), trs(1.0), Presence::Absent, Presence::Absent),
        NodeAuthoringNode::new(parent(1, 0), trs(2.0), Presence::Absent, Presence::Absent),
        NodeAuthoringNode::new(parent(1, 1), trs(3.0), Presence::Absent, Presence::Absent),
    ];
    let first = build_scene(&account, &first_roots, &first_nodes);
    let second = build_scene(&account, &second_roots, &second_nodes);
    let limits = snapshot_limits(16_384, 64);

    let first_snapshot =
        capture_node_scene(&first.scene, limits, None).expect("first partition snapshot");
    let second_snapshot =
        capture_node_scene(&second.scene, limits, None).expect("second partition snapshot");

    assert_eq!(first_snapshot.totals.node_count, 4);
    assert_eq!(first_snapshot.totals.root_count, 1);
    assert_eq!(first_snapshot.totals.edge_count, 3);
    assert_eq!(first_snapshot.totals, second_snapshot.totals);
    assert_ne!(first_snapshot.digest, second_snapshot.digest);
}

#[test]
fn empty_scene_snapshot_is_complete_and_deterministic() {
    let factory = factory();
    let first_account = account(&factory);
    let second_account = account(&factory);
    let first = build_scene(&first_account, &[], &[]);
    let second = build_scene(&second_account, &[], &[]);
    let limits = snapshot_limits(4_096, 64);

    let (first_snapshot, transcript) =
        capture_node_scene_with_test_transcript(&first.scene, limits, None)
            .expect("first empty snapshot");
    let second_snapshot =
        capture_node_scene(&second.scene, limits, None).expect("second empty snapshot");

    assert_eq!(first_snapshot.digest, second_snapshot.digest);
    assert_eq!(first_snapshot.preview, second_snapshot.preview);
    assert_eq!(first_snapshot.totals, second_snapshot.totals);
    assert_eq!(first_snapshot.totals.node_count, 0);
    assert_eq!(first_snapshot.totals.root_count, 0);
    assert_eq!(first_snapshot.totals.edge_count, 0);
    assert_eq!(first_snapshot.totals.image_count, 0);
    assert_eq!(first_snapshot.totals.sampler_count, 0);
    assert_eq!(first_snapshot.totals.texture_count, 0);
    assert_eq!(first_snapshot.totals.material_count, 0);
    assert_eq!(first_snapshot.totals.material_binding_count, 0);
    assert_eq!(first_snapshot.totals.logical_mesh_count, 0);
    assert_eq!(first_snapshot.totals.primitive_count, 0);
    assert_eq!(first_snapshot.totals.attribute_count, 0);
    assert_eq!(first_snapshot.totals.mesh_instance_count, 0);
    assert_eq!(first_snapshot.totals.polygon_boundary_count, 0);
    assert_eq!(first_snapshot.totals.primitive_index_count, 0);
    assert_eq!(first_snapshot.totals.sparse_index_count, 0);
    assert_eq!(first_snapshot.totals.canonical_scalar_count, 0);
    assert_eq!(first_snapshot.totals.skin_count, 0);
    assert_eq!(first_snapshot.totals.logical_morph_count, 0);
    assert_eq!(first_snapshot.totals.primitive_deformation_count, 0);
    assert_eq!(first_snapshot.totals.deformation_mesh_instance_count, 0);
    assert_eq!(first_snapshot.totals.animation_clip_count, 0);
    assert_eq!(first_snapshot.totals.animation_channel_count, 0);
    assert_eq!(first_snapshot.totals.profile_resource_count, 1);
    assert_eq!(first_snapshot.totals.profile_source_count, 1);
    assert_eq!(first_snapshot.totals.profile_provenance_node_count, 1);
    assert_eq!(first_snapshot.totals.profile_owner_count, 2);
    assert_eq!(first_snapshot.totals.profile_attachment_count, 2);
    assert_eq!(first_snapshot.totals.transcript_bytes, 858);
    assert_eq!(first_snapshot.totals.work_units, 87);
    assert!(transcript.complete);
    assert_hex_parts(
        &transcript.bytes,
        &[
            EMPTY_SCENE_TRANSCRIPT_PREFIX_HEX,
            EMPTY_PRE_PROFILE_DOMAINS_TRANSCRIPT_HEX,
            FRESH_PROFILE_TRANSCRIPT_PREFIX_HEX,
            PROFILE_OWNER_COLLECTION_2_HEX,
            PROFILE_SCENE_OWNER_ROW_HEX,
            PROFILE_SYNTHETIC_ROOT_OWNER_ROW_HEX,
            EMPTY_PROFILE_METADATA_TRANSCRIPT_HEX,
        ],
    );
}

#[test]
fn geometry_tables_payload_kinds_and_totals_are_complete() {
    let factory = factory();
    let account = account(&factory);
    let parts = all_attribute_payload_kinds_scene(&account);

    let snapshot = capture_node_scene(&parts.scene, snapshot_limits(32_768, 64), None)
        .expect("complete geometry snapshot");

    assert_eq!(snapshot.totals.material_count, 1);
    assert_eq!(snapshot.totals.logical_mesh_count, 1);
    assert_eq!(snapshot.totals.primitive_count, 3);
    assert_eq!(snapshot.totals.attribute_count, 6);
    assert_eq!(snapshot.totals.mesh_instance_count, 1);
    assert_eq!(snapshot.totals.polygon_boundary_count, 1);
    assert_eq!(snapshot.totals.primitive_index_count, 6);
    assert_eq!(snapshot.totals.sparse_index_count, 2);
    assert_eq!(snapshot.totals.canonical_scalar_count, 36);
    assert_eq!(snapshot.totals.f32_scalar_count, 33);
    assert_eq!(snapshot.totals.u32_scalar_count, 3);
    assert_eq!(snapshot.totals.profile_owner_count, 15);
    assert_eq!(snapshot.totals.profile_attachment_count, 15);
    assert_eq!(snapshot.totals.work_units, 306);
    assert!(snapshot.preview.text.contains("payload=dense"));
    assert!(snapshot.preview.text.contains("payload=sparse"));
    assert!(snapshot.preview.text.contains("sparse_default(present"));
    assert!(snapshot.preview.text.contains("polygon_boundaries(present"));
    assert!(
        snapshot
            .preview
            .text
            .contains("summary(nodes=1,roots=1,edges=0")
    );
    assert!(!snapshot.preview.bytes_truncated);
    assert!(!snapshot.preview.items_truncated);
}

#[test]
fn absent_and_present_empty_geometry_leaves_remain_distinct() {
    let factory = factory();
    let account = account(&factory);
    let absent = point_index_presence_scene(&account, false);
    let present_empty = point_index_presence_scene(&account, true);
    let limits = snapshot_limits(16_384, 64);

    let absent_snapshot =
        capture_node_scene(&absent.scene, limits, None).expect("absent-index snapshot");
    let present_empty_snapshot = capture_node_scene(&present_empty.scene, limits, None)
        .expect("present-empty-index snapshot");

    assert_ne!(absent_snapshot.digest, present_empty_snapshot.digest);
    assert_eq!(absent_snapshot.totals, present_empty_snapshot.totals);
    assert!(
        absent_snapshot
            .preview
            .text
            .contains("indices(absent,total=0)=[]")
    );
    assert!(
        present_empty_snapshot
            .preview
            .text
            .contains("indices(present,total=0)=[]")
    );
}

#[test]
fn projected_and_expanded_attributes_have_the_same_canonical_snapshot() {
    let factory = factory();
    let account = account(&factory);
    let projected_values = [0.0_f32, 0.0, 1.0, 0.0, 1.0, 0.0];
    let projection = [0_u32, 1, 0];
    let expanded_values = [0.0_f32, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let projected =
        corner_normal_scene(&account, &projected_values, Presence::Present(&projection));
    let expanded = corner_normal_scene(&account, &expanded_values, Presence::Absent);
    let limits = snapshot_limits(16_384, 64);

    let projected_snapshot =
        capture_node_scene(&projected.scene, limits, None).expect("projected snapshot");
    let expanded_snapshot =
        capture_node_scene(&expanded.scene, limits, None).expect("expanded snapshot");

    assert_eq!(projected_snapshot.digest, expanded_snapshot.digest);
    assert_eq!(projected_snapshot.preview, expanded_snapshot.preview);
    assert_eq!(projected_snapshot.totals, expanded_snapshot.totals);
    assert_eq!(projected_snapshot.totals.canonical_scalar_count, 18);
    assert_eq!(projected_snapshot.totals.f32_scalar_count, 18);
}

#[test]
fn geometry_mutation_after_preview_boundary_changes_complete_digest() {
    let factory = factory();
    let account = account(&factory);
    let left_positions = [1.0_f32, 2.0, 3.0];
    let right_positions = [1.0_f32, 2.0, 4.0];
    let left = point_geometry_scene(&account, &left_positions);
    let right = point_geometry_scene(&account, &right_positions);
    let limits = snapshot_limits(8_192, 1);

    let left_snapshot =
        capture_node_scene(&left.scene, limits, None).expect("left geometry snapshot");
    let right_snapshot =
        capture_node_scene(&right.scene, limits, None).expect("right geometry snapshot");

    assert_eq!(left_snapshot.preview, right_snapshot.preview);
    assert!(left_snapshot.preview.items_truncated);
    assert_ne!(left_snapshot.digest, right_snapshot.digest);
    assert_eq!(left_snapshot.totals, right_snapshot.totals);
}

#[test]
fn geometry_capture_honors_cancellation_and_exact_work_bounds() {
    let factory = factory();
    let account = account(&factory);
    let parts = all_attribute_payload_kinds_scene(&account);
    let retained = account.snapshot();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    assert!(matches!(
        capture_node_scene(
            &parts.scene,
            snapshot_limits(32_768, 64),
            Some(&cancellation)
        ),
        Err(NodeSceneSnapshotError::Cancelled)
    ));

    let full = capture_node_scene(&parts.scene, snapshot_limits(32_768, 64), None)
        .expect("full geometry work measurement");
    let exact = NodeSceneSnapshotLimits::try_new(32_768, 64, full.totals.work_units)
        .expect("exact geometry work limit");
    assert!(capture_node_scene(&parts.scene, exact, None).is_ok());
    let one_short = NodeSceneSnapshotLimits::try_new(
        32_768,
        64,
        full.totals
            .work_units
            .checked_sub(1)
            .expect("nonzero geometry snapshot work"),
    )
    .expect("one-short geometry work limit");
    assert!(matches!(
        capture_node_scene(&parts.scene, one_short, None),
        Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. })
    ));
    assert_eq!(account.snapshot(), retained);
}

#[test]
fn geometry_collection_fault_fails_before_a_snapshot_can_be_published() {
    let factory = factory();
    let account = account(&factory);
    let positions = [1.0_f32, 2.0, 3.0];
    let parts = point_geometry_scene(&account, &positions);
    let retained = account.snapshot();
    let source = parts.scene.semantic_view().expect("point semantic view");
    let mut nodes = source.into_cursor().expect("point semantic cursor");
    let mut roots = nodes.take_roots().expect("point roots");
    assert_eq!(roots.next().expect("point root"), Some(0));
    roots.finish().expect("complete point roots");
    let node = nodes
        .next_node()
        .expect("point node read")
        .expect("point node");
    node.into_children()
        .finish()
        .expect("complete point children");
    let appearance = nodes.appearance().expect("point appearance cursor");
    assert!(
        appearance
            .next_image()
            .expect("point image exhaustion")
            .is_none()
    );
    assert!(
        appearance
            .next_sampler()
            .expect("point sampler exhaustion")
            .is_none()
    );
    assert!(
        appearance
            .next_texture()
            .expect("point texture exhaustion")
            .is_none()
    );
    let material = appearance
        .next_material()
        .expect("point material read")
        .expect("point material");
    assert_eq!(material.source_index(), 0);
    assert!(
        appearance
            .next_material()
            .expect("point material exhaustion")
            .is_none()
    );
    assert!(
        appearance
            .next_binding()
            .expect("point binding exhaustion")
            .is_none()
    );
    let geometry = nodes.geometry().expect("point geometry cursor");
    let wrong_counts = super::GeometryCollectionCounts {
        logical_meshes: 2,
        primitives: 1,
        attributes: 1,
        mesh_instances: 1,
    };
    let mut transcript = super::CanonicalDigestEncoder::new();
    let mut preview = super::PreviewSink::new(4_096, 64).expect("fault preview");
    let mut work = SnapshotWorkMeter::new(MAX_WORK, None);

    let failure = super::write_geometry(
        geometry,
        wrong_counts,
        &mut transcript,
        &mut preview,
        &mut work,
    );
    assert!(matches!(
        failure,
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "logical-mesh collection ended before declared total"
        })
    ));
    assert_eq!(account.snapshot(), retained);
}

#[test]
fn mutation_after_preview_boundary_changes_only_the_complete_digest() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0), NodeLocalId::new(1)];
    let left_nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            trs(1.0),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            trs(2.0),
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let right_nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            trs(1.0),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            trs(3.0),
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let left = build_scene(&account, &roots, &left_nodes);
    let right = build_scene(&account, &roots, &right_nodes);
    let limits = snapshot_limits(4_096, 1);

    let left_snapshot =
        capture_node_scene(&left.scene, limits, None).expect("left bounded snapshot");
    let right_snapshot =
        capture_node_scene(&right.scene, limits, None).expect("right bounded snapshot");

    assert_eq!(left_snapshot.preview.text, right_snapshot.preview.text);
    assert_eq!(
        left_snapshot.preview.items_omitted_by_limit,
        right_snapshot.preview.items_omitted_by_limit
    );
    assert!(left_snapshot.preview.items_truncated);
    assert_ne!(left_snapshot.digest, right_snapshot.digest);
    assert_eq!(left_snapshot.totals, right_snapshot.totals);
}

#[test]
fn equivalent_semantics_ignore_local_indices_account_and_generation() {
    let factory = factory();
    let first_account = account(&factory);
    let second_account = account(&factory);

    let first_roots = [NodeLocalId::new(0)];
    let first_nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            matrix(4.0),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(parent(0, 0), trs(8.0), Presence::Absent, Presence::Absent),
    ];
    let second_roots = [NodeLocalId::new(1)];
    let second_nodes = [
        NodeAuthoringNode::new(parent(1, 0), trs(8.0), Presence::Absent, Presence::Absent),
        NodeAuthoringNode::new(
            Presence::Absent,
            matrix(4.0),
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let first = build_scene(&first_account, &first_roots, &first_nodes);
    let second = build_scene(&second_account, &second_roots, &second_nodes);

    assert!(
        !first
            .scene
            .account()
            .is_same_account(second.scene.account())
    );
    assert_ne!(first.scene.generation(), second.scene.generation());
    let first_snapshot = capture_node_scene(&first.scene, snapshot_limits(8_192, 64), None)
        .expect("first equivalent snapshot");
    let second_snapshot = capture_node_scene(&second.scene, snapshot_limits(8_192, 64), None)
        .expect("second equivalent snapshot");

    assert_eq!(first_snapshot.digest, second_snapshot.digest);
    assert_eq!(first_snapshot.preview, second_snapshot.preview);
    assert_eq!(first_snapshot.totals, second_snapshot.totals);
}

#[test]
fn sibling_order_representation_and_signed_zero_are_semantic() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0)];
    let ordered_nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            trs(0.0),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(parent(0, 0), trs(1.0), Presence::Absent, Presence::Absent),
        NodeAuthoringNode::new(parent(0, 1), trs(2.0), Presence::Absent, Presence::Absent),
    ];
    let swapped_nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            trs(0.0),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(parent(0, 1), trs(1.0), Presence::Absent, Presence::Absent),
        NodeAuthoringNode::new(parent(0, 0), trs(2.0), Presence::Absent, Presence::Absent),
    ];
    let ordered = build_scene(&account, &roots, &ordered_nodes);
    let swapped = build_scene(&account, &roots, &swapped_nodes);
    let positive_zero = single_root_scene(&account, trs(0.0));
    let negative_zero = single_root_scene(&account, trs(-0.0));
    let matrix_identity = single_root_scene(&account, matrix(0.0));
    let trs_identity = single_root_scene(&account, AuthoredTransform::identity_trs());
    let limits = snapshot_limits(16_384, 64);

    let digest = |parts: &NodeSceneSuccessParts| {
        capture_node_scene(&parts.scene, limits, None)
            .expect("semantic discriminator snapshot")
            .digest
    };
    assert_ne!(digest(&ordered), digest(&swapped));
    assert_ne!(digest(&positive_zero), digest(&negative_zero));
    assert_ne!(digest(&matrix_identity), digest(&trs_identity));
}

#[test]
fn proof_and_mid_traversal_read_failures_do_not_publish_partial_snapshots() {
    let factory = factory();
    let account = account(&factory);

    let mut layout_mismatch = single_root_scene(&account, AuthoredTransform::identity_trs());
    assert!(
        layout_mismatch
            .scene
            .install_semantic_layout_fault_for_test(2, 2)
    );
    let donor = single_root_scene(&account, trs(1.0));
    let mut foreign_proof = single_root_scene(&account, trs(2.0));
    foreign_proof
        .scene
        .install_foreign_semantic_proof_for_test(&donor.scene);

    let roots = [NodeLocalId::new(0)];
    let nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            trs(0.0),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(parent(0, 0), trs(1.0), Presence::Absent, Presence::Absent),
    ];
    let mut child_read_failure = build_scene(&account, &roots, &nodes);
    child_read_failure
        .scene
        .install_child_reference_read_fault_for_test(1);
    let mut transform_read_failure = build_scene(&account, &roots, &nodes);
    transform_read_failure
        .scene
        .install_authored_transform_read_fault_for_test(1);
    let retained = account.snapshot();

    assert!(matches!(
        capture_node_scene(&layout_mismatch.scene, snapshot_limits(4_096, 64), None),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "validated node table row count disagrees with validation proof"
        })
    ));
    assert!(matches!(
        capture_node_scene(&foreign_proof.scene, snapshot_limits(4_096, 64), None),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "validated node table is unavailable or mistyped"
        })
    ));
    assert!(matches!(
        capture_node_scene(&child_read_failure.scene, snapshot_limits(4_096, 64), None),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "injected child-reference read failure"
        })
    ));
    assert!(matches!(
        capture_node_scene(
            &transform_read_failure.scene,
            snapshot_limits(4_096, 64),
            None
        ),
        Err(NodeSceneSnapshotError::InconsistentView {
            reason: "injected authored-transform read failure"
        })
    ));

    assert_eq!(account.snapshot(), retained);
}

#[test]
fn limits_cancellation_and_capture_leave_physical_state_unchanged() {
    assert!(matches!(
        NodeSceneSnapshotLimits::try_new(MAX_PREVIEW_BYTES + 1, 1, 1),
        Err(NodeSceneSnapshotError::InvalidLimit {
            limit: NodeSceneSnapshotLimit::PreviewBytes,
            ..
        })
    ));
    assert!(matches!(
        NodeSceneSnapshotLimits::try_new(1, MAX_COLLECTION_ITEMS + 1, 1),
        Err(NodeSceneSnapshotError::InvalidLimit {
            limit: NodeSceneSnapshotLimit::CollectionItems,
            ..
        })
    ));
    assert!(matches!(
        NodeSceneSnapshotLimits::try_new(1, 1, MAX_WORK + 1),
        Err(NodeSceneSnapshotError::InvalidLimit {
            limit: NodeSceneSnapshotLimit::Work,
            ..
        })
    ));

    let factory = factory();
    let account = account(&factory);
    let parts = single_root_scene(&account, AuthoredTransform::identity_trs());
    let retained = account.snapshot();

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        capture_node_scene(&parts.scene, snapshot_limits(256, 4), Some(&cancellation)),
        Err(NodeSceneSnapshotError::Cancelled)
    ));
    let insufficient_work =
        NodeSceneSnapshotLimits::try_new(256, 4, 1).expect("bounded work configuration");
    assert!(matches!(
        capture_node_scene(&parts.scene, insufficient_work, None),
        Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. })
    ));
    let mut zero_work_result = None;
    let zero_work_allocations = allocation_counter::measure(|| {
        zero_work_result = Some(capture_node_scene(
            &parts.scene,
            NodeSceneSnapshotLimits::try_new(1, 1, 0).expect("zero work configuration"),
            None,
        ));
    });
    assert!(matches!(
        zero_work_result,
        Some(Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. }))
    ));
    assert_eq!(zero_work_allocations.count_total, 0);
    assert_eq!(zero_work_allocations.bytes_total, 0);

    for byte_limit in 1..=7 {
        let tiny = capture_node_scene(&parts.scene, snapshot_limits(byte_limit, 64), None)
            .expect("tiny preview limit remains a valid output bound");
        assert!(tiny.preview.text.len() <= byte_limit);
        assert!(tiny.preview.bytes_truncated);
        assert!(tiny.preview.text.capacity() >= byte_limit);
        assert!(tiny.preview.text.capacity() <= byte_limit.next_power_of_two());
    }
    let zero_preview = capture_node_scene(
        &parts.scene,
        NodeSceneSnapshotLimits::try_new(0, 0, MAX_WORK).expect("zero preview bounds"),
        None,
    )
    .expect("zero preview still captures complete digest");
    assert_eq!(zero_preview.digest, KNOWN_SINGLE_TRS_DIGEST);
    assert!(zero_preview.preview.text.is_empty());
    assert!(zero_preview.preview.bytes_truncated);
    assert!(zero_preview.preview.items_truncated);
    assert_eq!(account.snapshot(), retained);

    let full =
        capture_node_scene(&parts.scene, snapshot_limits(4_096, 64), None).expect("full snapshot");
    assert_eq!(full.digest, KNOWN_SINGLE_TRS_DIGEST);
    assert_eq!(account.snapshot(), retained);

    let exact_work = NodeSceneSnapshotLimits::try_new(4_096, 64, full.totals.work_units)
        .expect("exact work configuration");
    assert!(capture_node_scene(&parts.scene, exact_work, None).is_ok());
    let one_short = NodeSceneSnapshotLimits::try_new(
        4_096,
        64,
        full.totals
            .work_units
            .checked_sub(1)
            .expect("nonzero snapshot work"),
    )
    .expect("one-short work configuration");
    assert!(matches!(
        capture_node_scene(&parts.scene, one_short, None),
        Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. })
    ));
}

#[test]
fn work_meter_observes_cancellation_at_the_bounded_poll_interval() {
    let cancellation = CancellationToken::new();
    let mut work = SnapshotWorkMeter::new(MAX_WORK, Some(&cancellation));
    for _ in 1..super::NODE_SCENE_SNAPSHOT_POLL_INTERVAL {
        work.observe().expect("work before the poll boundary");
    }
    cancellation.cancel();
    assert!(matches!(
        work.observe(),
        Err(NodeSceneSnapshotError::Cancelled)
    ));
}

#[test]
fn traversal_scales_linearly_while_preview_stays_bounded() {
    let factory = factory();
    let account = account(&factory);
    let mut observed = Vec::new();

    for node_count in [1_usize, 10, 100, 1_000] {
        let roots = [NodeLocalId::new(0)];
        let nodes = (0..node_count)
            .map(|index| {
                let parent_ref = if index == 0 {
                    Presence::Absent
                } else {
                    parent(0, u32::try_from(index - 1).expect("test child ordinal"))
                };
                NodeAuthoringNode::new(
                    parent_ref,
                    AuthoredTransform::identity_trs(),
                    Presence::Absent,
                    Presence::Absent,
                )
            })
            .collect::<Vec<_>>();
        let parts = build_scene(&account, &roots, &nodes);
        let mut captured = None;
        let allocations = allocation_counter::measure(|| {
            captured = Some(capture_node_scene(
                &parts.scene,
                snapshot_limits(64, 1),
                None,
            ));
        });
        let snapshot = captured
            .expect("scale capture result")
            .expect("scale snapshot");

        assert!(snapshot.preview.text.len() <= 64);
        assert_eq!(
            snapshot.totals.transcript_bytes,
            858 + 151 * node_count as u64
        );
        assert_eq!(snapshot.totals.work_units, 87 + 22 * node_count as u64);
        assert_eq!(snapshot.totals.node_count, node_count);
        assert_eq!(snapshot.totals.root_count, 1);
        assert_eq!(snapshot.totals.edge_count, node_count - 1);
        assert_eq!(snapshot.totals.profile_owner_count, node_count + 2);
        assert_eq!(snapshot.totals.profile_attachment_count, node_count + 2);
        observed.push((
            snapshot.totals.transcript_bytes,
            snapshot.totals.work_units,
            allocations.count_total,
            allocations.bytes_total,
        ));
    }

    assert_eq!(observed[1].0 - observed[0].0, 9 * 151);
    assert_eq!(observed[2].0 - observed[0].0, 99 * 151);
    assert_eq!(observed[1].1 - observed[0].1, 9 * 22);
    assert_eq!(observed[2].1 - observed[0].1, 99 * 22);
    assert_eq!(observed[3].0 - observed[0].0, 999 * 151);
    assert_eq!(observed[3].1 - observed[0].1, 999 * 22);
    assert_eq!(observed[1].2, observed[0].2);
    assert_eq!(observed[2].2, observed[0].2);
    assert_eq!(observed[3].2, observed[0].2);
    assert_eq!(observed[1].3, observed[0].3);
    assert_eq!(observed[2].3, observed[0].3);
    assert_eq!(observed[3].3, observed[0].3);
}
