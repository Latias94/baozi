use super::*;
use crate::attribute::{
    Attribute, AttributeBuffer, AttributeDomain, AttributeFormat, AttributePayload,
    AttributeSemantic, ComponentType, Normalization,
};
use crate::canonical_validation::validate_canonical_scene;
use crate::deform::{
    CanonicalSkinDescriptor, MorphTargetDefinition, MorphWeightOverrideError, SkinMatrixSpaces,
};
use crate::geometry::{
    CanonicalMeshInstanceDescriptor, IndexStream, LogicalMesh, LogicalMeshBounds,
    MeshInstanceAttributeError, MeshInstanceError, MeshPrimitive, PrimitiveBounds, Topology,
};
use crate::identity::{LogicalMeshId, NodeId, SceneEntityKind, SceneLookupError, SkinId};
use crate::math::{Mat4, Vec3, Vec4};
use crate::validation_control::SceneValidationLimits;

fn positions(count: usize, provenance: Provenance) -> Attribute {
    let format = AttributeFormat::new(ComponentType::F32, 3, Normalization::None, None)
        .expect("plain position format should be valid");
    Attribute::new(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        format,
        AttributePayload::dense(
            AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO; count])
                .expect("position fixture storage should be valid"),
        ),
        provenance,
    )
    .expect("position fixture should be valid")
}

fn instance_color(provenance: Provenance) -> Attribute {
    let format = AttributeFormat::new(ComponentType::F32, 4, Normalization::None, None)
        .expect("plain instance color format should be valid");
    Attribute::new(
        AttributeSemantic::Color(0),
        AttributeDomain::Instance,
        format,
        AttributePayload::dense(
            AttributeBuffer::from_f32_vec4(vec![Vec4::new(1.0, 0.5, 0.25, 1.0)])
                .expect("instance color fixture storage should be valid"),
        ),
        provenance,
    )
    .expect("instance color fixture should be valid")
}

fn logical_mesh_with_morph_targets(
    provenance: Provenance,
    morph_target_count: usize,
) -> LogicalMesh {
    let primitive_morph_target_count = u32::try_from(morph_target_count)
        .expect("fixture morph-target count should fit canonical storage");
    let primitive = MeshPrimitive::try_new(
        3,
        Topology::TriangleList,
        IndexStream::non_indexed(),
        None,
        vec![positions(3, provenance)],
        None,
        Vec::new(),
        primitive_morph_target_count,
        PrimitiveBounds::default(),
        provenance,
    )
    .expect("morph primitive fixture should be valid");
    LogicalMesh::try_new(
        None,
        vec![primitive],
        (0..morph_target_count)
            .map(|_| {
                MorphTargetDefinition::try_new(None, provenance)
                    .expect("morph definition fixture should be valid")
            })
            .collect(),
        vec![0.0; morph_target_count],
        LogicalMeshBounds::default(),
        provenance,
    )
    .expect("morph logical mesh fixture should be valid")
}

fn produce_mesh_instance_references(
    builder: &mut CanonicalSceneBuilder,
    provenance: Provenance,
) -> (NodeId, LogicalMeshId, SkinId) {
    let owner = produce_nodes(builder, provenance, 1)[0];
    let logical_mesh = builder
        .try_push_logical_mesh(logical_mesh_with_morph_targets(provenance, 2))
        .expect("logical mesh should be stored");
    let skin = builder
        .try_push_skin(
            CanonicalSkinDescriptor::try_new(
                None,
                [owner],
                [Mat4::IDENTITY],
                None,
                SkinMatrixSpaces::canonical(),
                provenance,
            )
            .expect("skin descriptor fixture should be valid"),
        )
        .expect("skin should be stored");
    (owner, logical_mesh, skin)
}

#[test]
fn second_logical_mesh_id_round_trips_and_defines_the_instance_override_width() {
    let (mut builder, provenance) = scene_builder();
    let owner = produce_nodes(&mut builder, provenance, 1)[0];
    let first_mesh = builder
        .try_push_logical_mesh(logical_mesh_with_morph_targets(provenance, 1))
        .expect("first logical mesh should be stored");
    let second_mesh = builder
        .try_push_logical_mesh(logical_mesh_with_morph_targets(provenance, 2))
        .expect("second logical mesh should be stored");

    assert_eq!(first_mesh.generation(), builder.generation());
    assert_eq!(first_mesh.index(), 0);
    assert_eq!(second_mesh.generation(), builder.generation());
    assert_eq!(second_mesh.index(), 1);

    let first_instance = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            first_mesh,
            None,
            Some(vec![0.5]),
            Vec::new(),
            provenance,
        ))
        .expect("the first logical mesh should define a one-value override");
    let second_instance = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            second_mesh,
            None,
            Some(vec![0.25, -0.5]),
            Vec::new(),
            provenance,
        ))
        .expect("the second logical mesh should define a two-value override");

    let scene = validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
        .expect("both qualified logical-mesh references should validate");
    let first_view = scene
        .mesh_instance(first_instance)
        .expect("the first mesh instance should resolve");
    let second_view = scene
        .mesh_instance(second_instance)
        .expect("the second mesh instance should resolve");

    assert_eq!(first_view.logical_mesh(), first_mesh);
    assert_eq!(first_view.morph_weight_override(), Some([0.5].as_slice()));
    assert_eq!(second_view.logical_mesh(), second_mesh);
    assert_eq!(
        second_view.morph_weight_override(),
        Some([0.25, -0.5].as_slice())
    );
}

#[test]
fn second_mesh_instance_id_and_payload_round_trip_through_canonical_validation() {
    let (mut builder, provenance) = scene_builder();
    let (owner, logical_mesh, skin) = produce_mesh_instance_references(&mut builder, provenance);

    let first = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            logical_mesh,
            Some(skin),
            Some(vec![0.0, 0.0]),
            Vec::new(),
            provenance,
        ))
        .expect("first qualified mesh instance should be stored");
    let second = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            logical_mesh,
            Some(skin),
            Some(vec![0.25, -0.5]),
            Vec::new(),
            provenance,
        ))
        .expect("second qualified mesh instance should be stored");

    assert_eq!(first.generation(), builder.generation());
    assert_eq!(first.index(), 0);
    assert_eq!(second.generation(), builder.generation());
    assert_eq!(second.index(), 1);

    let scene = validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
        .expect("qualified mesh-instance fixture should validate");
    let second_view = scene
        .mesh_instance(second)
        .expect("the second mesh-instance id should resolve after validation");
    assert_eq!(second_view.owner(), owner);
    assert_eq!(second_view.logical_mesh(), logical_mesh);
    assert_eq!(second_view.skin(), Some(skin));
    assert_eq!(
        second_view.morph_weight_override(),
        Some([0.25, -0.5].as_slice())
    );
    assert!(second_view.attributes().is_empty());
    assert_eq!(second_view.provenance().first(), provenance);
}

#[test]
fn resolved_logical_mesh_defines_override_width_and_count_precedes_finite_validation() {
    let (mut builder, provenance) = scene_builder();
    let (owner, logical_mesh, skin) = produce_mesh_instance_references(&mut builder, provenance);
    let original_len = builder.data.mesh_instances.len();
    let original_capacity = builder.data.mesh_instances.capacity();

    let count_error = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            logical_mesh,
            Some(skin),
            Some(vec![f32::NAN]),
            Vec::new(),
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        count_error,
        CanonicalSceneBuildError::MeshInstance(MeshInstanceError::MorphWeightOverride(
            MorphWeightOverrideError::CountMismatch {
                expected: 2,
                actual: 1,
            },
        ))
    );
    assert_eq!(builder.data.mesh_instances.len(), original_len);
    assert_eq!(builder.data.mesh_instances.capacity(), original_capacity);

    let finite_error = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            logical_mesh,
            Some(skin),
            Some(vec![0.0, f32::NAN]),
            Vec::new(),
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        finite_error,
        CanonicalSceneBuildError::MeshInstance(MeshInstanceError::MorphWeightOverride(
            MorphWeightOverrideError::NonFiniteWeight { index: 1 },
        ))
    );
    assert_eq!(builder.data.mesh_instances.len(), original_len);
    assert_eq!(builder.data.mesh_instances.capacity(), original_capacity);
}

#[test]
fn mesh_instance_attributes_precede_override_validation() {
    let (mut builder, provenance) = scene_builder();
    let (owner, logical_mesh, skin) = produce_mesh_instance_references(&mut builder, provenance);
    let original_len = builder.data.mesh_instances.len();
    let original_capacity = builder.data.mesh_instances.capacity();

    let error = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            logical_mesh,
            Some(skin),
            Some(vec![f32::NAN]),
            vec![positions(1, provenance)],
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::MeshInstance(MeshInstanceError::Attributes(
            MeshInstanceAttributeError::InvalidDomain {
                attribute: 0,
                domain: AttributeDomain::Vertex,
            },
        ))
    );
    assert_eq!(builder.data.mesh_instances.len(), original_len);
    assert_eq!(builder.data.mesh_instances.capacity(), original_capacity);
}

#[test]
fn mesh_instance_checks_every_generation_before_any_storage_range() {
    let (foreign_owner_builder, _) = scene_builder();
    let foreign_owner_generation = foreign_owner_builder.generation();
    let (foreign_mesh_builder, _) = scene_builder();
    let foreign_mesh_generation = foreign_mesh_builder.generation();
    let (foreign_skin_builder, _) = scene_builder();
    let foreign_skin_generation = foreign_skin_builder.generation();

    let (mut builder, provenance) = scene_builder();
    produce_mesh_instance_references(&mut builder, provenance);
    let generation = builder.generation();
    let missing_owner = NodeId::new(generation, 1);
    let missing_mesh = LogicalMeshId::new(generation, 1);
    let original_len = builder.data.mesh_instances.len();
    let original_capacity = builder.data.mesh_instances.capacity();

    let owner_error = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            NodeId::new(foreign_owner_generation, u32::MAX),
            LogicalMeshId::new(foreign_mesh_generation, u32::MAX),
            Some(SkinId::new(foreign_skin_generation, u32::MAX)),
            Some(vec![f32::NAN]),
            Vec::new(),
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        owner_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Node,
            expected: generation,
            actual: foreign_owner_generation,
        })
    );
    assert_eq!(builder.data.mesh_instances.len(), original_len);
    assert_eq!(builder.data.mesh_instances.capacity(), original_capacity);

    let mesh_error = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            missing_owner,
            LogicalMeshId::new(foreign_mesh_generation, u32::MAX),
            Some(SkinId::new(foreign_skin_generation, u32::MAX)),
            Some(vec![f32::NAN]),
            Vec::new(),
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        mesh_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::LogicalMesh,
            expected: generation,
            actual: foreign_mesh_generation,
        })
    );
    assert_eq!(builder.data.mesh_instances.len(), original_len);
    assert_eq!(builder.data.mesh_instances.capacity(), original_capacity);

    let skin_error = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            missing_owner,
            missing_mesh,
            Some(SkinId::new(foreign_skin_generation, u32::MAX)),
            Some(vec![f32::NAN]),
            Vec::new(),
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        skin_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Skin,
            expected: generation,
            actual: foreign_skin_generation,
        })
    );
    assert_eq!(builder.data.mesh_instances.len(), original_len);
    assert_eq!(builder.data.mesh_instances.capacity(), original_capacity);
}

#[test]
fn mesh_instance_checks_owner_mesh_and_skin_ranges_in_order() {
    let (mut builder, provenance) = scene_builder();
    let (owner, logical_mesh, skin) = produce_mesh_instance_references(&mut builder, provenance);
    let generation = builder.generation();
    let missing_owner = NodeId::new(generation, 1);
    let missing_mesh = LogicalMeshId::new(generation, 1);
    let missing_skin = SkinId::new(generation, 1);
    let original_len = builder.data.mesh_instances.len();
    let original_capacity = builder.data.mesh_instances.capacity();

    let owner_error = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            missing_owner,
            missing_mesh,
            Some(missing_skin),
            Some(vec![f32::NAN]),
            Vec::new(),
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        owner_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Node,
            length: 1,
        })
    );
    assert_eq!(builder.data.mesh_instances.len(), original_len);
    assert_eq!(builder.data.mesh_instances.capacity(), original_capacity);

    let mesh_error = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            missing_mesh,
            Some(missing_skin),
            Some(vec![f32::NAN]),
            Vec::new(),
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        mesh_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::LogicalMesh,
            length: 1,
        })
    );
    assert_eq!(builder.data.mesh_instances.len(), original_len);
    assert_eq!(builder.data.mesh_instances.capacity(), original_capacity);

    let skin_error = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            logical_mesh,
            Some(missing_skin),
            Some(vec![f32::NAN]),
            Vec::new(),
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        skin_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Skin,
            length: 1,
        })
    );
    assert_eq!(builder.data.mesh_instances.len(), original_len);
    assert_eq!(builder.data.mesh_instances.capacity(), original_capacity);

    let instance = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            logical_mesh,
            Some(skin),
            Some(vec![0.0, 0.0]),
            Vec::new(),
            provenance,
        ))
        .expect("all references should resolve once each handle enters range");
    assert_eq!(instance.generation(), generation);
    assert_eq!(instance.index(), 0);
    assert_eq!(builder.data.mesh_instances.len(), 1);
}

#[test]
fn prepared_mesh_instance_never_publishes_before_commit_and_remains_retryable() {
    let (mut builder, provenance) = scene_builder();
    let (owner, logical_mesh, skin) = produce_mesh_instance_references(&mut builder, provenance);
    let generation = builder.generation();
    let plan = builder
        .collection_capacity_plan(CanonicalSceneCollection::MeshInstances)
        .expect("mesh-instance insertion should have a capacity plan");
    let collection_admitted = plan
        .collection_envelope_delta_bytes()
        .expect("mesh-instance collection envelope should fit");
    let value_admitted = plan.value_storage_envelope_bytes();

    let prepared = builder
        .prepare_push_mesh_instance(
            CanonicalMeshInstanceDescriptor::new(
                owner,
                logical_mesh,
                Some(skin),
                Some(vec![0.25, -0.5]),
                Vec::new(),
                provenance,
            ),
            plan,
        )
        .expect("valid mesh instance should prepare off to the side");
    assert!(prepared.replaces_collection_allocation());
    assert!(prepared.collection_retained_allocation_bytes() <= collection_admitted);
    assert!(prepared.value_storage_retained_allocation_bytes() <= value_admitted);
    drop(prepared);
    assert!(builder.data.mesh_instances.is_empty());
    assert_eq!(builder.data.mesh_instances.capacity(), 0);

    let instance = builder
        .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
            owner,
            logical_mesh,
            Some(skin),
            Some(vec![0.25, -0.5]),
            Vec::new(),
            provenance,
        ))
        .expect("dropping a prepared mesh instance must leave the builder reusable");
    assert_eq!(instance.generation(), generation);
    assert_eq!(instance.index(), 0);
    assert_eq!(builder.data.mesh_instances.len(), 1);
}

#[test]
fn prepared_mesh_instances_discard_hostile_override_and_attribute_envelopes() {
    let (mut builder, provenance) = scene_builder();
    let (owner, logical_mesh, skin) = produce_mesh_instance_references(&mut builder, provenance);

    let mut override_weights = Vec::with_capacity(1_024);
    override_weights.extend([0.25, -0.5]);
    let hostile_override_bytes = override_weights.capacity() * std::mem::size_of::<f32>();
    let override_plan = builder
        .collection_capacity_plan(CanonicalSceneCollection::MeshInstances)
        .expect("override fixture should have a capacity plan");
    let prepared_override = builder
        .prepare_push_mesh_instance(
            CanonicalMeshInstanceDescriptor::new(
                owner,
                logical_mesh,
                Some(skin),
                Some(override_weights),
                Vec::new(),
                provenance,
            ),
            override_plan,
        )
        .expect("hostile override capacity should prepare after compaction");
    assert!(
        prepared_override.value_storage_retained_allocation_bytes() < hostile_override_bytes as u64
    );
    let first = prepared_override.commit();
    assert_eq!(first.index(), 0);
    assert!(
        builder.data.mesh_instances[0]
            .as_ref()
            .retained_footprint()
            .expect("stored override footprint should fit")
            .bytes()
            < hostile_override_bytes
    );

    let mut attributes = Vec::with_capacity(1_024);
    attributes.push(instance_color(provenance));
    let hostile_attribute_bytes = attributes.capacity() * std::mem::size_of::<Attribute>();
    let attribute_plan = builder
        .collection_capacity_plan(CanonicalSceneCollection::MeshInstances)
        .expect("attribute fixture should have a capacity plan");
    let prepared_attribute = builder
        .prepare_push_mesh_instance(
            CanonicalMeshInstanceDescriptor::new(
                owner,
                logical_mesh,
                Some(skin),
                Some(vec![0.25, -0.5]),
                attributes,
                provenance,
            ),
            attribute_plan,
        )
        .expect("hostile attribute capacity should prepare after compaction");
    assert!(
        prepared_attribute.value_storage_retained_allocation_bytes()
            < hostile_attribute_bytes as u64
    );
    let second = prepared_attribute.commit();
    assert_eq!(second.index(), 1);
    assert!(
        builder.data.mesh_instances[1]
            .as_ref()
            .retained_footprint()
            .expect("stored attribute footprint should fit")
            .bytes()
            < hostile_attribute_bytes
    );
}
